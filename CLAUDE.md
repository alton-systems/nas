# CLAUDE.md — nas

Instruções para agentes (humanos ou IA) trabalhando neste repositório.

## O que é o nas

Editor de texto de terminal em que a IA é um usuário de primeira classe: ela
entende, usa e reescreve o próprio editor, com cada mudança tipada, permitida
e reversível. Ver `README.md` para os quatro pilares e o roadmap completo.

**Marco atual: Marco 1 — editor básico utilizável + modal do nas-keys. Sem IA,
sem Lisp.** Não adiante trabalho dos marcos 2 a 4 (Steel, `defcommand`,
permissões por efeito, propostas da IA) enquanto o marco 1 não estiver pronto.

## Decisões fixas (não reabrir sem discutir)

- **Linguagem:** Rust, edition 2024, definida uma vez em
  `[workspace.package]` no `Cargo.toml` raiz e herdada nas crates
  (`edition.workspace = true`, `rust-version.workspace = true`).
- **helix-core/helix-lsp/helix-loader como dependências git, fixadas numa tag
  de release** (atualmente `25.07.1`). Nunca usar `helix-term` ou
  `helix-view`. Isolar o uso dessas crates atrás dos nossos módulos — a API
  delas não é estável. Ver `docs/decisions/0001-helix-como-biblioteca.md`.
- **Terminal:** `ratatui` + `crossterm`, com o protocolo de teclado do Kitty
  quando disponível e fallback quando não.
- **Assíncrono:** `tokio`. **Logs:** `tracing`, só em arquivo, nunca na tela
  do TUI. **Testes:** `cargo test` + `insta` para snapshots de tela.
- **Licença:** MPL-2.0 no projeto inteiro. Ver
  `docs/decisions/0002-licenca-mpl.md`.
- **O sistema de teclas é o nas-keys** (https://github.com/2gab/nas-keys,
  arquivos `nas.el`/`nas-state.el`/`nas-map.el`/`nas-layout.el`). O
  repositório do nas-keys é a fonte da verdade — ele continua mudando, e
  diverge do que está documentado aqui. Antes de tocar em `nas-keys` (crate),
  clone e leia a versão atual do nas-keys. Atalhos, estados, prefixos e
  despachos vivem como **dados declarativos** (ex.:
  `crates/nas-keys/keymap/default.toml`), nunca espalhados pelo código Rust.
  Ver `docs/decisions/0003-nas-keys-como-keymap.md`.

## Estrutura do workspace

```
crates/
  nas-term/    # binário: interface ratatui/crossterm, loop de eventos
  nas-editor/  # estado do editor: documentos, cursores, transações (usa helix-core)
  nas-keys/    # máquina de estados modal e tradução de layout
docs/decisions/  # ADRs — uma nota curta por decisão importante
spikes/          # protótipos descartáveis, fora do workspace principal
```

Crates futuras (não criar antes do marco correspondente): `nas-lisp`
(marco 2), `nas-commands` (marco 3), `nas-ai` (marco 4).

## Regras de trabalho

- **Uma etapa do roadmap por vez.** Ao fim de cada etapa: `cargo fmt`,
  `cargo clippy -- -D warnings`, `cargo test`, commits pequenos e
  descritivos, resumo curto do que mudou e como testar na mão — depois
  **parar e esperar o ok** antes da próxima etapa.
- **Cada etapa numa branch própria** (ex.: `etapa-5b`), terminando com
  um Pull Request aberto no GitHub com um resumo da etapa. Push na
  branch da etapa está liberado. **Nunca fazer push direto na `main`
  nem merge do próprio PR** — quem revisa e faz o merge é o autor.
- Antes de uma decisão grande que não esteja registrada (trocar uma
  biblioteca, mudar a estrutura de crates), parar e perguntar.
- Se uma API do `helix-core` (ou `helix-lsp`/`helix-loader`) não estiver
  clara, ler o código-fonte da tag fixada em vez de adivinhar — essas crates
  não têm documentação estável.
- Preferir código legível e simples a código esperto. Sem abstrações
  antecipando marcos futuros.
- Manter este arquivo atualizado quando uma regra ou decisão mudar.
