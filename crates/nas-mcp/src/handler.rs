//! O `ServerHandler` que fala MCP sobre o registro de `nas-commands`
//! — a única coisa neste crate que conhece `rmcp` diretamente (ADR
//! 0006-ia-trabalha-pelo-nas.md §3, decisão 0001: isolar a crate
//! instável atrás do nosso próprio adaptador).

use std::collections::HashSet;
use std::sync::Mutex;

use rmcp::ErrorData as McpError;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParam, CallToolResult, Content, Implementation, ListToolsResult,
    PaginatedRequestParam, ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer};

use nas_commands::{CommandContext, CommandSpec, Effect, Registry, Workspace};
use nas_history::Author;

use crate::options::McpOptions;

pub(crate) struct NasMcpHandler {
    registry: Registry,
    workspace: Mutex<Workspace>,
    approve_writes: bool,
    /// Gerado uma vez, ao iniciar — `Author::Ai::session` pra toda
    /// chamada desta conexão (ADR 0006 §6).
    session_id: String,
}

impl NasMcpHandler {
    pub(crate) fn new(options: McpOptions) -> Self {
        let mut workspace = Workspace::new(&options.project_root);
        if let Some(history_dir) = &options.history_dir {
            workspace = workspace.with_history_dir(history_dir);
            match nas_history::Journal::open(history_dir) {
                Ok(journal) => workspace = workspace.with_journal(journal),
                Err(err) => tracing::warn!(error = %err, "não foi possível abrir o journal"),
            }
        }

        let mut registry = Registry::new();
        nas_commands::register_all(&mut registry);

        NasMcpHandler {
            registry,
            workspace: Mutex::new(workspace),
            approve_writes: options.approve_writes,
            session_id: session_id(),
        }
    }

    /// `agent` vem do `clientInfo` que o cliente MCP manda no
    /// handshake (ADR 0006 §6) — "mcp" se, por algum motivo, o
    /// handshake ainda não aconteceu (não deveria, `rmcp` já exige
    /// `initialize` antes de `tools/call`).
    fn author(&self, context: &RequestContext<RoleServer>) -> Author {
        let agent = context
            .peer
            .peer_info()
            .map(|info| info.client_info.name.clone())
            .unwrap_or_else(|| "mcp".to_string());
        Author::Ai {
            agent,
            session: self.session_id.clone(),
        }
    }

    /// Efeitos já aprovados pra esta chamada — vazio, a menos que
    /// `--approve-writes` tenha liberado `WriteFile` (nunca
    /// `Shell`/`Network`: ADR 0006, revisão da etapa 3c).
    fn approved_effects(&self) -> HashSet<Effect> {
        let mut approved = HashSet::new();
        if self.approve_writes {
            approved.insert(Effect::WriteFile);
        }
        approved
    }
}

fn session_id() -> String {
    format!("{}-{}", nas_history::now_unix(), std::process::id())
}

/// O resultado de sucesso de uma ferramenta, como blocos de conteúdo
/// MCP — texto puro pra quem devolve texto de arquivo de verdade
/// (`read`/`search`/`list_files`/`show_diff`: linhas numeradas, como
/// a IA já vê em qualquer um desses), nunca esse texto dentro de um
/// JSON com quebras de linha escapadas (`\n` literal na string, mais
/// difícil de ler que o texto de verdade). Os outros comandos
/// continuam como JSON — são metadados pequenos (`path`, `from`/`to`),
/// sem motivo pra um formato especial. Um aviso (`sync_with_disk`/
/// `open_or_resync`), quando presente, vai num segundo bloco de
/// conteúdo, nunca misturado ao primeiro.
fn render_success(name: &str, value: &serde_json::Value) -> Vec<Content> {
    let mut blocks = match name {
        "read" => vec![Content::text(text_field(value, "text"))],
        "search" => vec![Content::text(render_matches(value))],
        "list_files" => vec![Content::text(render_file_list(value))],
        "show_diff" => vec![Content::text(render_diffs(value))],
        _ => return vec![Content::text(value.to_string())],
    };
    if let Some(warning) = value["warning"].as_str() {
        blocks.push(Content::text(warning.to_string()));
    }
    blocks
}

fn text_field(value: &serde_json::Value, field: &str) -> String {
    value[field].as_str().unwrap_or_default().to_string()
}

fn render_matches(value: &serde_json::Value) -> String {
    value["matches"]
        .as_array()
        .map(|matches| {
            matches
                .iter()
                .map(|m| {
                    format!(
                        "{} {}",
                        m["line"].as_u64().unwrap_or_default(),
                        m["text"].as_str().unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn render_file_list(value: &serde_json::Value) -> String {
    value["files"]
        .as_array()
        .map(|files| {
            files
                .iter()
                .filter_map(|f| f.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn render_diffs(value: &serde_json::Value) -> String {
    value["diffs"]
        .as_array()
        .map(|diffs| {
            diffs
                .iter()
                .map(|d| {
                    format!(
                        "## {} ({})\n{}",
                        d["path"].as_str().unwrap_or_default(),
                        d["change"].as_str().unwrap_or_default(),
                        d["unified"].as_str().unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .unwrap_or_default()
}

/// `nas-commands::CommandSpec` → `rmcp::model::Tool` — a lista de
/// ferramentas **é** o registro filtrado por `exposed_to_ai` (ADR
/// 0006 §5/ADR 0007-configuracao-em-camadas.md §5), nunca uma
/// segunda lista mantida à mão.
fn spec_to_tool(spec: &CommandSpec) -> Tool {
    let input_schema = spec.args_schema.as_object().cloned().unwrap_or_default();
    Tool {
        name: spec.name.into(),
        description: Some(spec.description.into()),
        input_schema: std::sync::Arc::new(input_schema),
        annotations: annotations_for(spec.effect),
    }
}

/// Dica (nunca garantia — `ToolAnnotations` é só isso, uma dica pro
/// cliente) derivada do mesmo `Effect` que já decide aprovação (ADR
/// 0006 §5): `Pure`/`ReadFile` nunca mudam nada
/// (`read_only_hint`); `WriteFile` toca o disco de verdade, o que
/// `ToolAnnotations` chama de destrutivo (`destructive_hint`) — não
/// porque apague algo sem avisar (`save_file` recusa antes disso,
/// §5), mas porque sobrescreve o arquivo por completo. Os demais
/// efeitos (`WriteBuffer`, `WriteHistory`, `Shell`, `Network`) não
/// têm uma dica óbvia nenhuma das duas categorias, então ficam sem
/// anotação — nenhuma lista paralela, direto do mesmo enum.
fn annotations_for(effect: Effect) -> Option<ToolAnnotations> {
    match effect {
        Effect::Pure | Effect::ReadFile => Some(ToolAnnotations::new().read_only(true)),
        Effect::WriteFile => Some(ToolAnnotations::new().destructive(true)),
        Effect::WriteBuffer | Effect::WriteHistory | Effect::Shell | Effect::Network => None,
    }
}

impl ServerHandler for NasMcpHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: "nas-mcp".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tools = self
            .registry
            .specs()
            .iter()
            .filter(|spec| spec.exposed_to_ai)
            .map(spec_to_tool)
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let exists_and_exposed = self
            .registry
            .specs()
            .iter()
            .any(|spec| spec.exposed_to_ai && spec.name == request.name);
        if !exists_and_exposed {
            return Err(McpError::invalid_params(
                format!("ferramenta desconhecida: {:?}", request.name),
                None,
            ));
        }

        let author = self.author(&context);
        let approved_effects = self.approved_effects();
        let args = request
            .arguments
            .map(serde_json::Value::Object)
            .unwrap_or_else(|| serde_json::Value::Object(Default::default()));

        let Ok(mut workspace) = self.workspace.lock() else {
            return Err(McpError::internal_error(
                "estado interno corrompido (lock envenenado)",
                None,
            ));
        };
        let mut ctx = CommandContext {
            workspace: &mut workspace,
            author,
            approved_effects,
        };

        match self.registry.run(&request.name, &mut ctx, args) {
            Ok(value) => Ok(CallToolResult::success(render_success(
                request.name.as_ref(),
                &value,
            ))),
            Err(err) => Ok(CallToolResult::error(vec![Content::text(err.to_string())])),
        }
    }
}
