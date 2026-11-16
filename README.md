# nas

Um editor de texto para o terminal em que a IA é um usuário de primeira classe: ela entende, usa e reescreve o próprio editor, e cada mudança dela é tipada, permitida e reversível. Nos outros editores a IA edita arquivos; no nas ela edita o editor, com contrato, permissão e desfazer. O sistema de teclas é o [nas-keys](https://github.com/2gab/nas-keys).

## Os quatro pilares

1. **Todo comando é um contrato.** Nome, argumentos tipados, docstring e efeitos (`pure`, `read-file`, `write-buffer`, `write-file`, `shell`, `network`) — de onde saem validação, ajuda, atalho e o JSON Schema que a IA recebe.
2. **Permissão pelo efeito.** Comandos `pure` rodam direto; os demais pedem aprovação. O interpretador só entrega a um comando as primitivas dos efeitos que ele declarou.
3. **Mudanças da IA são transações.** Tudo que a IA avalia vira um changeset: ver o diff, desfazer, ou promover com commit no Git.
4. **Uma linguagem de edição que humano e IA falam igual.** A IA propõe uma "frase" de comandos, exibida como no estado Sequential do nas-keys, e o usuário lê antes de executar.

## Roadmap

- **Marco 1 (em andamento):** editor básico utilizável + modal do nas-keys. Sem IA, sem Lisp.
- **Marco 2:** Steel (Scheme) embutido; uma primitiva do núcleo chamada a partir do Lisp.
- **Marco 3:** `defcommand` com esquema, docstring e efeitos; geração automática de JSON Schema; atalhos e ajuda derivados do registro.
- **Marco 4:** a IA propõe uma sequência de comandos, o usuário vê no estilo Sequential e aprova, a transação aplica e pode ser desfeita.

## Estrutura

```
crates/
  nas-term/    # binário: interface ratatui/crossterm, loop de eventos
  nas-editor/  # estado do editor: documentos, cursores, transações (usa helix-core)
  nas-keys/    # máquina de estados modal e tradução de layout
docs/decisions/  # ADRs
```

Decisões de arquitetura ficam registradas em `docs/decisions/`. Regras de trabalho para agentes/IA no repositório estão em `CLAUDE.md`.

## Instalação

```sh
cargo install --path crates/nas-term --locked
```

Isso instala o binário como `nas`.

## Uso

```sh
nas caminho/do/arquivo.txt
```

Durante o desenvolvimento, sem instalar:

```sh
cargo run -p nas-term -- caminho/do/arquivo.txt
```

Os atalhos seguem o [nas-keys](https://github.com/2gab/nas-keys) — ver
`crates/nas-keys/keymap/default.toml` para a lista completa. São
declarados por posição física num teclado US qwerty; se o seu teclado
for outro layout, defina `NAS_KEYBOARD_LAYOUT` antes de rodar o nas para
que as teclas continuem na posição certa:

```sh
export NAS_KEYBOARD_LAYOUT=qwerty-abnt  # ABNT2 — onde o qwerty tem ';', o ABNT2 tem 'ç'
```

Sem a variável (ou com um valor desconhecido), o nas usa `qwerty`, o
padrão do próprio nas-keys. Layouts disponíveis: `qwerty`, `qwerty-abnt`.

## Licença

MPL-2.0. Veja `LICENSE`.
