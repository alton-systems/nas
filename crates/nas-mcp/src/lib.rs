//! Adaptador MCP sobre o registro de `nas-commands` — ADR
//! 0006-ia-trabalha-pelo-nas.md §3. `rmcp` fica isolado atrás deste
//! crate (decisão 0001: a mesma regra de isolar crates instáveis já
//! aplicada a `helix-core`), nenhum outro crate do workspace o
//! importa diretamente.
//!
//! Transporte stdio só: nada além do protocolo MCP pode ir pro
//! stdout — quem chama `run_stdio` (hoje, `nas --mcp` em
//! `nas-term`) é responsável por manter logging em arquivo e nunca
//! restaurar o terminal nesse modo.

mod handler;
mod options;

pub use options::McpOptions;

use std::io;

use rmcp::ServiceExt;
use rmcp::transport::stdio;

use handler::NasMcpHandler;

pub async fn run_stdio(options: McpOptions) -> io::Result<()> {
    let handler = NasMcpHandler::new(options);
    let service = handler
        .serve(stdio())
        .await
        .map_err(|err| io::Error::other(err.to_string()))?;
    service
        .waiting()
        .await
        .map_err(|err| io::Error::other(err.to_string()))?;
    Ok(())
}
