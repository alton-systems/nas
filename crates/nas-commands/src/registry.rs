use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::context::CommandContext;
use crate::effect::Effect;
use crate::error::CommandError;

type RunFn =
    Box<dyn Fn(&mut CommandContext, serde_json::Value) -> Result<serde_json::Value, CommandError>>;

/// Nome, descrição, efeito e esquema de argumentos de um comando —
/// os quatro metadados do pilar 1 do README — mais a função que o
/// executa, por trás de um `serde_json::Value` dos dois lados, pra
/// que o `Registry` não precise saber o tipo concreto de cada
/// comando. Construído por `typed_command`, nunca à mão.
pub struct CommandSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub effect: Effect,
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
    f: F,
) -> CommandSpec
where
    A: DeserializeOwned + JsonSchema,
    R: Serialize,
    F: Fn(&mut CommandContext, A) -> Result<R, CommandError> + 'static,
{
    CommandSpec {
        name,
        description,
        effect,
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
        spec.run(ctx, args)
    }
}
