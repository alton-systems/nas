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

type EffectForArgsFn = Box<dyn Fn(&serde_json::Value) -> Effect + Send + Sync>;

/// Nome, descrição, efeito e esquema de argumentos de um comando —
/// os quatro metadados do pilar 1 do README — mais a função que o
/// executa, por trás de um `serde_json::Value` dos dois lados, pra
/// que o `Registry` não precise saber o tipo concreto de cada
/// comando. Construído por `typed_command`, nunca à mão.
pub struct CommandSpec {
    pub name: &'static str,
    pub description: &'static str,
    /// O efeito "de repouso" — o que `effective_effect` devolve
    /// quando o comando não tem efeito dependente de argumento
    /// (`effect_for_args`, abaixo). Pra a maioria dos comandos, é o
    /// único efeito que existe.
    pub effect: Effect,
    /// O efeito mais arriscado que esta ferramenta pode ter, entre
    /// todos os argumentos possíveis — igual a `effect`, a menos que
    /// `with_dynamic_effect` diga o contrário. Só usado pra decidir a
    /// anotação MCP estática de `tools/list` (`nas-mcp`), que
    /// precisa de uma resposta antes de qualquer argumento de uma
    /// chamada existir — escolhida de forma conservadora (o "pior
    /// caso"), nunca pra decidir aprovação de verdade (ADR 0006 §5,
    /// etapa 3f-prep: isso é sempre `effective_effect`).
    pub max_effect: Effect,
    /// Se este comando aparece como ferramenta pra um agente de IA
    /// (ADR 0006 §5/ADR 0007-configuracao-em-camadas.md §5): estar
    /// no registro **não basta** — cada comando precisa marcar isto
    /// explicitamente, por isso `typed_command` exige o valor, nunca
    /// um padrão. `nas-mcp` (etapa 3c) filtra por aqui pra montar a
    /// lista de ferramentas; nunca mantém uma segunda lista.
    pub exposed_to_ai: bool,
    pub args_schema: schemars::Schema,
    run: RunFn,
    /// Calcula o efeito **desta chamada específica**, a partir dos
    /// argumentos crus (ainda não validados contra o esquema — só
    /// uma leitura otimista de um campo booleano, nunca algo que
    /// decida segurança por si só: a validação de verdade continua
    /// sendo o `Deserialize` de `A` dentro de `run`, que recusa a
    /// chamada inteira, gravação incluída, se o argumento não bater
    /// com o tipo esperado). `None` pra todo comando cujo efeito não
    /// depende de argumento nenhum (a maioria) — `effective_effect`
    /// cai pra `effect` nesse caso.
    effect_for_args: Option<EffectForArgsFn>,
}

impl CommandSpec {
    pub fn run(
        &self,
        ctx: &mut CommandContext,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, CommandError> {
        (self.run)(ctx, args)
    }

    /// O efeito desta chamada específica, a partir dos argumentos
    /// crus que ela recebeu — o que `Registry::run` usa pra decidir
    /// aprovação (ADR 0006 §5) e `nas-mcp` usa pra decidir aprovação
    /// e registrar a decisão. Nunca `effect`/`max_effect` direto,
    /// pra um comando como `edit` (etapa 3f-prep: `save: true` muda o
    /// efeito de `WriteBuffer` pra `WriteFile`).
    pub fn effective_effect(&self, args: &serde_json::Value) -> Effect {
        match &self.effect_for_args {
            Some(f) => f(args),
            None => self.effect,
        }
    }

    /// Marca este comando como tendo um efeito dependente dos
    /// argumentos (ADR 0006 §5, etapa 3f-prep) — `max_effect` passa a
    /// ser o que `nas-mcp` anuncia em `tools/list` (o "pior caso",
    /// escolhido conservadoramente); `f` passa a decidir o efeito de
    /// cada chamada, substituindo o `effect` fixo original.
    pub fn with_dynamic_effect(
        mut self,
        max_effect: Effect,
        f: impl Fn(&serde_json::Value) -> Effect + Send + Sync + 'static,
    ) -> Self {
        self.max_effect = max_effect;
        self.effect_for_args = Some(Box::new(f));
        self
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
        max_effect: effect,
        exposed_to_ai,
        args_schema: schemars::schema_for!(A),
        run: Box::new(move |ctx, value| {
            let args: A = serde_json::from_value(value).map_err(CommandError::InvalidArgs)?;
            let result = f(ctx, args)?;
            serde_json::to_value(result).map_err(CommandError::Serialize)
        }),
        effect_for_args: None,
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
        // O efeito **desta chamada** — não `spec.effect` direto, pra
        // um comando como `edit` (ADR 0006 §5, etapa 3f-prep: `save:
        // true` muda o efeito de `WriteBuffer` pra `WriteFile`).
        // Decidido aqui, antes de rodar qualquer pedaço do comando —
        // é por isto que não existe jeito de `edit(save: true)`
        // gravar no disco sem passar pela aprovação de `write-file`:
        // este é o único caminho de execução, usado por `nas-mcp` e
        // por qualquer chamador direto (como os testes).
        let effect = spec.effective_effect(&args);
        if needs_approval(effect)
            && !matches!(ctx.author, Author::User)
            && !ctx.approved_effects.contains(&effect)
        {
            return Err(CommandError::NeedsApproval {
                command: name.to_string(),
                effect,
            });
        }

        // Capturado **antes** de rodar o comando — ADR 0006 §6, etapa
        // 3e: o checkpoint "antes da sessão" precisa do estado de
        // antes, nunca do que `spec.run` já tiver mudado (importa pra
        // `edit`, que já mutou o buffer na hora em que isto seria
        // computado de novo depois). Só é de fato criado (gravado em
        // disco) se o comando terminar com sucesso — nunca numa
        // tentativa que falhou (não foi uma mudança efetiva).
        let session_checkpoint = prepare_session_checkpoint(ctx, effect);

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
