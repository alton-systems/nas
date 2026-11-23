use std::path::PathBuf;

/// O que `nas --mcp` precisa pra iniciar o adaptador.
pub struct McpOptions {
    /// A raiz do projeto — descoberta do mesmo jeito que o histórico
    /// (ADR 0005 §6), não por conta própria deste crate.
    pub project_root: PathBuf,
    /// A pasta de histórico do projeto, se puder ser criada/aberta
    /// (`nas_history::ProjectId::ensure_history_dir`) — `None` sem
    /// histórico disponível; os comandos que precisam dele (`show_diff`,
    /// `checkpoint_create`, `checkpoint_restore`) recusam nesse caso,
    /// mas o servidor ainda sobe.
    pub history_dir: Option<PathBuf>,
    /// `--approve-writes` (ADR 0006-ia-trabalha-pelo-nas.md, revisão
    /// da etapa 3c): libera só `Effect::WriteFile`, nunca
    /// `Shell`/`Network` — temporário, só pra testar antes da
    /// aprovação de verdade (etapa 3d); remover ou substituir lá.
    /// Desligada por padrão.
    pub approve_writes: bool,
}
