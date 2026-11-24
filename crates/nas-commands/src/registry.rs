use std::path::PathBuf;

use nas_history::Author;
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::context::CommandContext;
use crate::effect::Effect;
use crate::error::CommandError;

type RunFn = Box<
    dyn Fn(&mut CommandContext, serde_json::Value) -> Result<serde_json::Value, CommandError>
        + Send
        + Sync,
>;

/// Nome, descrição, efeito e esquema de argumentos de um comando —
/// os quatro metadados do pilar 1 do README — mais a função que o
/// executa, por trás de um `serde_json::Value` dos dois lados, pra
/// que o `Registry` não precise saber o tipo concreto de cada
/// comando. Construído por `typed_command`, nunca à mão.
pub struct CommandSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub effect: Effect,
    /// Se este comando aparece como ferramenta pra um agente de IA
    /// (ADR 0006 §5/ADR 0007-configuracao-em-camadas.md §5): estar
    /// no registro **não basta** — cada comando precisa marcar isto
    /// explicitamente, por isso `typed_command` exige o valor, nunca
    /// um padrão. `nas-mcp` (etapa 3c) filtra por aqui pra montar a
    /// lista de ferramentas; nunca mantém uma segunda lista.
    pub exposed_to_ai: bool,
    pub args_schema: schemars::Schema,
    run: RunFn,
}

impl CommandSpec {
    pub fn run(
        &self,
        ctx: &mut CommandContext,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, CommandError> {
        (self.run)(ctx, args)
    }
}

/// Constrói um `CommandSpec` a partir de uma função Rust tipada — o
/// esquema JSON de `A` vem de `schemars`, nunca escrito à mão; quem
/// chama só vê `serde_json::Value` dos dois lados (`Registry::run`),
/// o que já é o formato que um agente de IA manda/recebe via MCP
/// (etapa 3c).
pub fn typed_command<A, R, F>(
    name: &'static str,
    description: &'static str,
    effect: Effect,
    exposed_to_ai: bool,
    f: F,
) -> CommandSpec
where
    A: DeserializeOwned + JsonSchema,
    R: Serialize,
    F: Fn(&mut CommandContext, A) -> Result<R, CommandError> + Send + Sync + 'static,
{
    CommandSpec {
        name,
        description,
        effect,
        exposed_to_ai,
        args_schema: schemars::schema_for!(A),
        run: Box::new(move |ctx, value| {
            let args: A = serde_json::from_value(value).map_err(CommandError::InvalidArgs)?;
            let result = f(ctx, args)?;
            serde_json::to_value(result).map_err(CommandError::Serialize)
        }),
    }
}

/// O registro de comandos — a lista de ferramentas que um agente de
/// IA vê via MCP **é** este registro (ADR 0006 §3), não uma segunda
/// lista mantida à mão.
#[derive(Default)]
pub struct Registry(Vec<CommandSpec>);

impl Registry {
    pub fn new() -> Self {
        Registry(Vec::new())
    }

    pub fn register(&mut self, spec: CommandSpec) {
        self.0.push(spec);
    }

    pub fn specs(&self) -> &[CommandSpec] {
        &self.0
    }

    pub fn get(&self, name: &str) -> Option<&CommandSpec> {
        self.0.iter().find(|spec| spec.name == name)
    }

    pub fn run(
        &self,
        name: &str,
        ctx: &mut CommandContext,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, CommandError> {
        let spec = self
            .get(name)
            .ok_or_else(|| CommandError::UnknownCommand(name.to_string()))?;
        if needs_approval(spec.effect)
            && !matches!(ctx.author, Author::User)
            && !ctx.approved_effects.contains(&spec.effect)
        {
            return Err(CommandError::NeedsApproval {
                command: name.to_string(),
                effect: spec.effect,
            });
        }

        // Capturado **antes** de rodar o comando — ADR 0006 §6, etapa
        // 3e: o checkpoint "antes da sessão" precisa do estado de
        // antes, nunca do que `spec.run` já tiver mudado (importa pra
        // `edit`, que já mutou o buffer na hora em que isto seria
        // computado de novo depois). Só é de fato criado (gravado em
        // disco) se o comando terminar com sucesso — nunca numa
        // tentativa que falhou (não foi uma mudança efetiva).
        let session_checkpoint = prepare_session_checkpoint(ctx, spec.effect);

        let mut result = spec.run(ctx, args);

        if result.is_ok()
            && let Some((history_dir, checkpoint_name, author, files)) = session_checkpoint
            && let Err(err) = nas_history::create_checkpoint(
                &history_dir,
                &checkpoint_name,
                author,
                nas_history::now_unix(),
                true,
                &files,
            )
        {
            ctx.warnings.push(format!(
                "não foi possível criar o checkpoint automático da sessão: {err}"
            ));
        }

        // Flush incondicional ao fim de toda chamada, aqui — um só
        // lugar, nunca em cada comando que journala — pra que a
        // autoria da IA nunca fique só na fila em memória se o
        // processo (nas-mcp, hoje sem núcleo compartilhado) for
        // encerrado à força entre uma chamada e a próxima. Sem custo
        // quando não há journal (`journal_mut` devolve `None`) ou a
        // fila está vazia (`Journal::flush` só escreve se tiver algo).
        //
        // O erro de um flush nunca é ignorado em silêncio (issue #24)
        // quando há um resultado de sucesso pra anexar o aviso — se o
        // próprio comando já tinha falhado, a falha do flush (as duas
        // ao mesmo tempo, um caso raríssimo) só vai pro retorno do
        // comando, sem canal pra mais um aviso ali.
        if let Some(journal) = ctx.workspace.journal_mut()
            && let Err(err) = journal.flush(nas_history::now_unix())
            && let Ok(value) = &mut result
        {
            append_warning(
                value,
                format!("não foi possível gravar o histórico em disco: {err}"),
            );
        }

        if let Ok(value) = &mut result {
            for warning in ctx.warnings.drain(..) {
                append_warning(value, warning);
            }
        }

        result
    }
}

/// Acrescenta `message` ao campo `"warning"` do resultado serializado
/// de um comando — combinando com um aviso que já estivesse lá (ex.:
/// `edit`/`open_file`/`read` já têm o próprio, de
/// `Workspace::sync_with_disk`/`open_or_resync`), nunca substituindo.
/// Sem efeito se `value` não for um objeto JSON (não deveria
/// acontecer — todo `CommandSpec::run` serializa uma struct).
fn append_warning(value: &mut serde_json::Value, message: String) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    let combined = match obj.get("warning").and_then(|w| w.as_str()) {
        Some(existing) if !existing.is_empty() => format!("{existing}\n{message}"),
        _ => message,
    };
    obj.insert("warning".to_string(), serde_json::Value::String(combined));
}

/// O que falta pra criar o checkpoint automático de sessão, se o
/// comando terminar com sucesso: onde (`history_dir`), o nome, o
/// autor, e o conteúdo de todo arquivo já rastreado (caminho,
/// bytes) — o mesmo formato que `nas_history::create_checkpoint`
/// espera.
type SessionCheckpointPlan = (PathBuf, String, Author, Vec<(String, Vec<u8>)>);

/// Se esta é a primeira chamada efetiva de `(agent, session)` com um
/// efeito que de fato muda algo (ADR 0006 §6, etapa 3e): captura, pra
/// criar depois do sucesso, o checkpoint automático "antes da sessão
/// de `<agent>`" — todo arquivo já rastreado (buffer aberto, se
/// houver; senão o disco), do mesmo jeito que `checkpoint_create`
/// (§1). `None` se não se aplica: efeito que não muda nada
/// (`Pure`/`ReadFile`/`WriteHistory`), autor que não é `Author::Ai`
/// (só sessão de IA tem esse checkpoint — ADR 0006 §6), sem
/// `history_dir`, ou a sessão já tem um (nunca repetido).
fn prepare_session_checkpoint(
    ctx: &CommandContext,
    effect: Effect,
) -> Option<SessionCheckpointPlan> {
    if !matches!(effect, Effect::WriteBuffer | Effect::WriteFile) {
        return None;
    }
    let Author::Ai { agent, .. } = &ctx.author else {
        return None;
    };
    let history_dir = ctx.workspace.history_dir()?.to_path_buf();
    if session_checkpoint_exists(&history_dir, &ctx.author) {
        return None;
    }
    let files = nas_history::list_tracked(&history_dir)
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| {
            let content = ctx.workspace.current_content_for(&entry.path)?;
            Some((entry.path.clone(), content))
        })
        .collect();
    let name = format!(
        "antes da sessão de {agent} ({})",
        nas_history::format_timestamp(nas_history::now_unix())
    );
    Some((history_dir, name, ctx.author.clone(), files))
}

/// Já existe um checkpoint automático desta `(agent, session)` —
/// "um por sessão, nunca repetido" (ADR 0006 §6).
fn session_checkpoint_exists(history_dir: &std::path::Path, author: &Author) -> bool {
    nas_history::list_checkpoints(history_dir)
        .unwrap_or_default()
        .iter()
        .any(|c| c.automatic && &c.author == author)
}

/// Efeitos que pedem aprovação pelo risco real (ADR 0006 §5):
/// `Pure`, `ReadFile`, `WriteBuffer` e `WriteHistory` rodam direto —
/// só `WriteFile`, `Shell` e `Network` tocam o disco de verdade ou
/// saem do projeto, e só quando quem chama **não** é o usuário
/// (`Author::User` — ações dele, editando direto, nunca pedem
/// aprovação de si mesmas). A checagem vale pra `Author::Ai` **e**
/// pra qualquer outro autor que não seja `User` — `Extension` e
/// `External` incluídos, de propósito: uma extensão futura (um
/// formatador, uma code action) nasce bloqueada por padrão, igual a
/// um agente de IA, em vez de precisar de uma exceção própria o dia
/// que existir. Quem decide se o efeito está de fato liberado
/// (política do projeto, etapa 3d) é quem chama `Registry::run`, não
/// este `Registry` — `ctx.approved_effects` já precisa ter o efeito
/// antes desta chamada.
fn needs_approval(effect: Effect) -> bool {
    matches!(effect, Effect::WriteFile | Effect::Shell | Effect::Network)
}
