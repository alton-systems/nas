//! Configuração em camadas — docs/decisions/0007-configuracao-em-camadas.md.
//! Camada 1 (dados, TOML) só: `config.toml` (opções de comportamento,
//! este módulo) e `keymap.toml` (atalhos, `crate::keymap_layer`),
//! lidos e validados num lugar só, nunca espalhados por `nas-term`.

mod config;
mod error;
mod keymap_layer;
mod loader;
mod paths;
mod spec;
mod suggest;
mod toml_parse;
mod value;

pub use config::Config;
pub use error::{ConfigError, format_errors};
pub use loader::{Loaded, load};
pub use paths::{config_home, project_config_path, user_config_dir};
