# 0004 — a especificação de teclas passa a viver no nas

## Status

Aceita e implementada (redesenho "teclas como ideogramas"). Substitui
`0003-nas-keys-como-keymap.md` no ponto em que ele dizia que o
repositório nas-keys é a fonte da verdade.

## Contexto

A ADR 0003 estabeleceu que o pacote de Emacs
[nas-keys](https://github.com/2gab/nas-keys) era a especificação dos
atalhos do nas, e que o `crates/nas-keys/keymap/default.toml` era uma
tradução dela pra dados em Rust — sincronizada manualmente, lendo o
diff do nas-keys a cada mudança.

Esse arranjo funcionou enquanto o nas só replicava o que o nas-keys já
tinha. Ele para de funcionar quando o design das teclas em si muda de
forma mais profunda que um atalho — como o redesenho "teclas como
ideogramas" (radicais, classes, verbo + objeto genérico), que não
existia no nas-keys e nasceu no próprio nas. Não dá pra "ler o diff"
de uma mudança que ainda não aconteceu no outro lado.

## Decisão

1. **A especificação das teclas passa a viver em `docs/keys.md`**,
   dentro do repositório do nas. Esse arquivo descreve a ideia central
   (radicais e compostos), o glossário, as regras, as classes, a
   tabela completa de teclas e o que ainda não foi implementado.
2. **`crates/nas-keys/keymap/default.toml` implementa `docs/keys.md`**,
   não o nas-map.el. Mudar uma tecla começa no `.md`, não no `.toml`
   — ver a seção "Como alterar este sistema" em `docs/keys.md`.
3. **O pacote de Emacs nas-keys não é alterado por esta decisão.**
   Ele continua existindo e sendo usado por quem edita com Emacs; a
   atualização dele a partir de `docs/keys.md` é um trabalho separado,
   feito depois, no repositório dele.
4. **A tradução de layout físico** (`crates/nas-keys/src/layout.rs`,
   decisão 3 do ADR anterior) não muda: continua uma tabela de dados,
   independente de onde a especificação das teclas mora.

## Consequências

- O nas e o nas-keys podem divergir deliberadamente por um tempo: o
  nas tem o redesenho, o nas-keys (Emacs) ainda não. Isso é esperado e
  temporário — a atualização do nas-keys é uma tarefa futura, não uma
  dívida silenciosa, porque agora está documentada aqui.
- Testes de comportamento continuam em `crates/nas-keys/src/
  machine.rs` e `keymap.rs`, exercitando `default.toml` através de
  `Keymap::default_keymap` — isso não muda.
- Decisões futuras sobre teclas (uma tecla nova, um verbo novo) se
  registram atualizando `docs/keys.md` diretamente, não com uma ADR
  nova — ADRs são pra decisões de arquitetura, não pra cada atalho.
