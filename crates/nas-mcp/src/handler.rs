//! O `ServerHandler` que fala MCP sobre o registro de `nas-commands`
//! — a única coisa neste crate que conhece `rmcp` diretamente (ADR
//! 0006-ia-trabalha-pelo-nas.md §3, decisão 0001: isolar a crate
//! instável atrás do nosso próprio adaptador).

use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use rmcp::ErrorData as McpError;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParam, CallToolResult, Content, Implementation, ListToolsResult,
    PaginatedRequestParam, ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer};

use nas_commands::{CommandContext, CommandSpec, Effect, Registry, Workspace};
use nas_history::{
    Author, PermissionDecision, PermissionDecisionEntry, PermissionSource, PermissionsPolicy,
};

use crate::options::McpOptions;

pub(crate) struct NasMcpHandler {
    registry: Registry,
    workspace: Mutex<Workspace>,
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

    /// A decisão pelo risco real (ADR 0006 §5): liberado pela política
    /// do projeto, ou negado — direto, se a política simplesmente não
    /// libera o efeito; com o motivo, se `permissions.toml` for
    /// inválido. Não há ainda pergunta interativa nem "sempre nesta
    /// sessão" (etapa 3d, revisão abaixo): o `rmcp` 0.3 fixado neste
    /// workspace não suporta `elicitation` (adicionado só em versões
    /// bem posteriores, major 1.0+), então o único canal de aprovação
    /// desta etapa é o arquivo.
    ///
    /// **Lê `permissions.toml` de novo em toda chamada** (revisão
    /// pedida na revisão do PR), nunca um valor cacheado desde
    /// `NasMcpHandler::new` — editar o arquivo enquanto `nas --mcp`
    /// está rodando (liberar um efeito, ou corrigir um erro de
    /// sintaxe) tem efeito já na próxima chamada, sem reiniciar o
    /// processo. O custo é uma leitura pequena de disco, só nas
    /// chamadas que já pedem aprovação (nunca em `Pure`/`ReadFile`/
    /// etc.) — desprezível frente ao resto de uma chamada MCP.
    fn decide_effect(&self, effect: Effect, history_dir: Option<&Path>) -> EffectDecision {
        let permissions = match history_dir {
            Some(history_dir) => nas_history::load_permissions(history_dir),
            None => Ok(PermissionsPolicy::default()),
        };
        match &permissions {
            Ok(policy) if nas_commands::allowed_by_policy(effect, policy) => {
                EffectDecision::Allowed
            }
            Ok(_) => EffectDecision::Denied { reason: None },
            Err(err) => {
                tracing::warn!(error = %err, "permissions.toml inválido — negando tudo até ser corrigido");
                EffectDecision::Denied {
                    reason: Some(err.to_string()),
                }
            }
        }
    }

    /// Acrescenta a decisão ao registro do projeto (ADR 0006 §5,
    /// "Histórico") — sem efeito se não houver `history_dir`, ou se o
    /// autor não for um agente de IA (nunca acontece aqui: `author`
    /// vem sempre de [`NasMcpHandler::author`], que só produz
    /// `Author::Ai`; o padrão continua explícito porque `Author` tem
    /// outros variants que não fazem sentido aqui).
    fn log_decision(
        &self,
        history_dir: Option<&Path>,
        author: &Author,
        command: &str,
        policy_key: &str,
        decision: &EffectDecision,
    ) {
        let (Author::Ai { agent, session }, Some(history_dir)) = (author, history_dir) else {
            return;
        };
        let entry = PermissionDecisionEntry {
            ts: nas_history::now_unix(),
            agent: agent.clone(),
            session: session.clone(),
            command: command.to_string(),
            effect: policy_key.to_string(),
            decision: decision.to_log(),
        };
        if let Err(err) = nas_history::record_permission_decision(history_dir, &entry) {
            tracing::warn!(error = %err, "não foi possível registrar a decisão de permissão");
        }
    }
}

/// O resultado de [`NasMcpHandler::decide_effect`] — separado de
/// `nas_history::PermissionDecision` (o que vai pro journal, ADR 0006
/// §5) porque só este lado carrega o motivo de uma recusa (pra
/// `deny_message`); o journal não precisa dele.
#[derive(Debug)]
enum EffectDecision {
    Allowed,
    Denied { reason: Option<String> },
}

impl EffectDecision {
    /// Hoje, toda aprovação vem do arquivo — não há pergunta nem
    /// sessão ainda (ver `NasMcpHandler::decide_effect`).
    fn to_log(&self) -> PermissionDecision {
        match self {
            EffectDecision::Allowed => PermissionDecision::Allowed {
                source: PermissionSource::File,
            },
            EffectDecision::Denied { .. } => PermissionDecision::Denied,
        }
    }
}

/// A mensagem que o agente recebe quando um efeito não está liberado
/// — sempre dizendo qual efeito falta e como liberar (`nas
/// --permissions`, ADR 0006 §5). Quando o motivo é um
/// `permissions.toml` inválido (não "simplesmente não liberado"), diz
/// isso também — tudo negado até o arquivo ser corrigido (ADR 0007
/// §3).
fn deny_message(command: &str, effect: Effect, invalid_policy_reason: Option<&str>) -> String {
    let key = effect.policy_key().unwrap_or("desconhecido");
    match invalid_policy_reason {
        Some(reason) => format!(
            "{command:?} tem efeito {effect:?} ({key:?}), que pede aprovação — o arquivo de política deste projeto é inválido, por isso nada é liberado: {reason}. Rode `nas --permissions` pra ver o caminho e corrigir."
        ),
        None => format!(
            "{command:?} tem efeito {effect:?} ({key:?}), que pede aprovação e não está liberado neste projeto — rode `nas --permissions` pra ver ou criar o arquivo de política e liberar {key:?}."
        ),
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
        let Some(spec) = self
            .registry
            .get(request.name.as_ref())
            .filter(|spec| spec.exposed_to_ai)
        else {
            return Err(McpError::invalid_params(
                format!("ferramenta desconhecida: {:?}", request.name),
                None,
            ));
        };

        let author = self.author(&context);
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
        let history_dir = workspace.history_dir().map(|dir| dir.to_path_buf());

        // Decidido aqui, antes de chamar o `Registry` — pra um efeito
        // negado nunca rodar nenhum pedaço do comando, e pra poder
        // devolver uma mensagem mais específica que o cinto de
        // segurança genérico de `Registry::run` (ADR 0006 §5, etapa
        // 3d).
        let mut approved_effects = HashSet::new();
        if let Some(policy_key) = spec.effect.policy_key() {
            let decision = self.decide_effect(spec.effect, history_dir.as_deref());
            self.log_decision(
                history_dir.as_deref(),
                &author,
                request.name.as_ref(),
                policy_key,
                &decision,
            );
            match decision {
                EffectDecision::Allowed => {
                    approved_effects.insert(spec.effect);
                }
                EffectDecision::Denied { reason } => {
                    return Ok(CallToolResult::error(vec![Content::text(deny_message(
                        request.name.as_ref(),
                        spec.effect,
                        reason.as_deref(),
                    ))]));
                }
            }
        }

        let mut ctx = CommandContext {
            workspace: &mut workspace,
            author,
            approved_effects,
            warnings: Vec::new(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn handler(project_root: &Path, history_dir: Option<&Path>) -> NasMcpHandler {
        NasMcpHandler::new(McpOptions {
            project_root: project_root.to_path_buf(),
            history_dir: history_dir.map(Path::to_path_buf),
        })
    }

    #[test]
    fn sem_historico_e_sem_arquivo_nega_sem_motivo() {
        let project = tempfile::tempdir().unwrap();
        let h = handler(project.path(), None);

        let decision = h.decide_effect(Effect::WriteFile, None);
        assert!(matches!(decision, EffectDecision::Denied { reason: None }));
    }

    #[test]
    fn arquivo_ausente_nega_sem_motivo() {
        let project = tempfile::tempdir().unwrap();
        let history = tempfile::tempdir().unwrap();
        let h = handler(project.path(), Some(history.path()));

        let decision = h.decide_effect(Effect::WriteFile, Some(history.path()));
        assert!(matches!(decision, EffectDecision::Denied { reason: None }));
    }

    #[test]
    fn arquivo_liberando_o_efeito_aprova() {
        let project = tempfile::tempdir().unwrap();
        let history = tempfile::tempdir().unwrap();
        std::fs::write(
            nas_history::permissions_path(history.path()),
            "write-file = true\n",
        )
        .unwrap();
        let h = handler(project.path(), Some(history.path()));

        assert!(matches!(
            h.decide_effect(Effect::WriteFile, Some(history.path())),
            EffectDecision::Allowed
        ));
        // Um efeito que o arquivo não cita continua negado — liberar
        // `write-file` não libera `shell`/`network` de brinde.
        assert!(matches!(
            h.decide_effect(Effect::Shell, Some(history.path())),
            EffectDecision::Denied { reason: None }
        ));
    }

    #[test]
    fn arquivo_invalido_nega_com_motivo() {
        let project = tempfile::tempdir().unwrap();
        let history = tempfile::tempdir().unwrap();
        std::fs::write(nas_history::permissions_path(history.path()), "voar = 1\n").unwrap();
        let h = handler(project.path(), Some(history.path()));

        match h.decide_effect(Effect::WriteFile, Some(history.path())) {
            EffectDecision::Denied { reason: Some(_) } => {}
            other => panic!("devia negar com motivo: {other:?}"),
        }
    }

    /// Revisão do PR: `permissions.toml` precisa ser relido em toda
    /// chamada, não só uma vez ao iniciar — editar o arquivo com o
    /// processo já rodando (liberar um efeito antes negado) tem efeito
    /// já na próxima decisão, sem reiniciar `nas --mcp`.
    #[test]
    fn mudanca_no_arquivo_depois_de_iniciar_o_processo_e_vista_na_proxima_chamada() {
        let project = tempfile::tempdir().unwrap();
        let history = tempfile::tempdir().unwrap();
        let h = handler(project.path(), Some(history.path()));

        assert!(matches!(
            h.decide_effect(Effect::WriteFile, Some(history.path())),
            EffectDecision::Denied { reason: None }
        ));

        std::fs::write(
            nas_history::permissions_path(history.path()),
            "write-file = true\n",
        )
        .unwrap();

        assert!(matches!(
            h.decide_effect(Effect::WriteFile, Some(history.path())),
            EffectDecision::Allowed
        ));
    }

    #[test]
    fn arquivo_dentro_do_projeto_e_ignorado() {
        let project = tempfile::tempdir().unwrap();
        let history = tempfile::tempdir().unwrap();
        // Um `permissions.toml` dentro do projeto (nunca em
        // `history_dir`) liberando tudo — tem que ser ignorado por
        // completo: permissão nunca pode vir de dentro do repositório.
        std::fs::write(
            project.path().join("permissions.toml"),
            "write-file = true\nshell = true\nnetwork = true\n",
        )
        .unwrap();
        let h = handler(project.path(), Some(history.path()));

        assert!(matches!(
            h.decide_effect(Effect::WriteFile, Some(history.path())),
            EffectDecision::Denied { reason: None }
        ));
    }

    #[test]
    fn decisoes_vao_pro_journal_de_permissoes() {
        let project = tempfile::tempdir().unwrap();
        let history = tempfile::tempdir().unwrap();
        std::fs::write(
            nas_history::permissions_path(history.path()),
            "write-file = true\n",
        )
        .unwrap();
        let h = handler(project.path(), Some(history.path()));
        let author = Author::Ai {
            agent: "teste".to_string(),
            session: "s1".to_string(),
        };

        let allowed = h.decide_effect(Effect::WriteFile, Some(history.path()));
        h.log_decision(
            Some(history.path()),
            &author,
            "save_file",
            "write-file",
            &allowed,
        );
        let denied = h.decide_effect(Effect::Shell, Some(history.path()));
        h.log_decision(Some(history.path()), &author, "run_shell", "shell", &denied);

        let entries = nas_history::read_permission_decisions(history.path()).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].command, "save_file");
        assert_eq!(
            entries[0].decision,
            PermissionDecision::Allowed {
                source: PermissionSource::File
            }
        );
        assert_eq!(entries[1].command, "run_shell");
        assert_eq!(entries[1].decision, PermissionDecision::Denied);
    }

    #[test]
    fn mensagem_de_recusa_cita_o_efeito_e_nas_permissions() {
        let message = deny_message("save_file", Effect::WriteFile, None);
        assert!(message.contains("save_file"));
        assert!(message.contains("nas --permissions"));
        assert!(message.contains("write-file"));
    }

    #[test]
    fn mensagem_de_recusa_com_arquivo_invalido_cita_o_motivo() {
        let message = deny_message("save_file", Effect::WriteFile, Some("TOML inválido: x"));
        assert!(message.contains("inválido"));
        assert!(message.contains("TOML inválido: x"));
    }
}
