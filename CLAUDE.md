# CLAUDE.md — nas

Instruções para agentes (humanos ou IA) trabalhando neste repositório.

## O que é o nas

Editor de texto de terminal em que a IA é um usuário de primeira classe: ela
entende, usa e reescreve o próprio editor, com cada mudança tipada, permitida
e reversível. Ver `README.md` para os quatro pilares e o roadmap completo.

**Marco atual: fase 3 — "a IA trabalha pelo nas"
(`docs/decisions/0006-ia-trabalha-pelo-nas.md`), em andamento pelas etapas
3a-3f da própria ADR.** O marco 1 (editor básico + modal do nas-keys) e a
fase 2 (histórico com autoria, `docs/decisions/0005`) estão prontos. A ADR
0006 inverteu a ordem original do roteiro abaixo: um registro de comandos
nativo em Rust (`nas-commands`) e um servidor MCP chegam antes de qualquer
Lisp/Steel embutido — ver a ADR pelo porquê. Não adiante trabalho de Lisp
embutido ou ACP (ambos adiados, não cancelados, pela própria ADR 0006)
enquanto a fase 3 não estiver pronta, a menos que uma ADR própria decida o
contrário.

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
- **A especificação das teclas é `docs/keys.md`**, no próprio nas — a
  ideia dos radicais e compostos (verbo + objeto, unidades, classes
  por modificador), as regras, a tabela completa e o que falta. O
  pacote de Emacs [nas-keys](https://github.com/2gab/nas-keys) deixou
  de ser a fonte da verdade (ver
  `docs/decisions/0004-especificacao-de-teclas-no-nas.md`); ele
  continua existindo e será atualizado a partir de `docs/keys.md`,
  não o contrário. Antes de tocar em `nas-keys` (crate), leia
  `docs/keys.md`. Atalhos, estados, prefixos, verbos, unidades e
  objetos vivem como **dados declarativos** (ex.:
  `crates/nas-keys/keymap/default.toml`), nunca espalhados pelo
  código Rust — a lógica em `machine.rs` só pode ser genérica (válida
  pra qualquer verbo/unidade/prefixo), nunca uma tecla específica
  hardcoded. Ver `docs/decisions/0003-nas-keys-como-keymap.md` e
  `0004-especificacao-de-teclas-no-nas.md`.

## Estrutura do workspace

```
crates/
  nas-term/      # binário: interface ratatui/crossterm, loop de eventos
  nas-editor/    # estado do editor: documentos, cursores, transações (usa helix-core)
  nas-keys/      # máquina de estados modal e tradução de layout
  nas-history/   # histórico com autoria, journal, checkpoints (ADR 0005)
  nas-commands/  # registro de comandos tipados — nome, args, efeito (ADR 0006 §1)
  nas-config/    # configuração em camadas: config.toml, keymap.toml (ADR 0007)
  nas-mcp/       # adaptador MCP (rmcp isolado) sobre nas-commands; nas --mcp (ADR 0006 §3)
docs/decisions/  # ADRs — uma nota curta por decisão importante
spikes/          # protótipos descartáveis, fora do workspace principal
```

`nas-lisp`/`nas-ai`, do roteiro original do README, ficam sem data —
a ADR 0006 adiou o Lisp embutido, não cancelou.

## Regras de trabalho

- **Uma etapa do roadmap por vez.** Ao fim de cada etapa: `cargo fmt`,
  `cargo clippy -- -D warnings`, `cargo test`, commits pequenos e
  descritivos, resumo curto do que mudou e como testar na mão — depois
  **parar e esperar o ok** antes da próxima etapa.
- **Cada etapa numa branch própria** (ex.: `etapa-5b`), terminando com
  um Pull Request aberto no GitHub com um resumo da etapa. Push na
  branch da etapa está liberado. **Nunca fazer push direto na `main`
  nem merge do próprio PR** — quem revisa e faz o merge é o autor. Essa
  regra vale pra toda mudança, inclusive de processo e documentação
  (como esta própria linha) — nenhuma exceção por ser "só" um ajuste
  pequeno.
- Antes de uma decisão grande que não esteja registrada (trocar uma
  biblioteca, mudar a estrutura de crates), parar e perguntar.
- Se uma API do `helix-core` (ou `helix-lsp`/`helix-loader`) não estiver
  clara, ler o código-fonte da tag fixada em vez de adivinhar — essas crates
  não têm documentação estável.
- Preferir código legível e simples a código esperto. Sem abstrações
  antecipando marcos futuros.
- Manter este arquivo atualizado quando uma regra ou decisão mudar.
