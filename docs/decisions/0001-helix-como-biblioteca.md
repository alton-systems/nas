# 0001 — helix-core como biblioteca, não como fork

## Status

Aceita.

## Contexto

Precisamos de rope, transações, histórico (undo/redo), seleções, Tree-sitter
e LSP. Escrever tudo isso do zero seria um projeto em si; o Helix já resolve
bem esses problemas. A alternativa seria dar fork no Helix e adaptar a UI,
mas o sistema de comandos do `helix-term` é um `enum` estático — o oposto do
que o nas precisa (comandos tipados, com efeitos, descobertos em runtime para
a IA).

## Decisão

Usar `helix-core` (e, quando necessário, `helix-lsp` e `helix-loader`) como
dependências git do repositório oficial
(https://github.com/helix-editor/helix), **fixadas numa tag de release**, não
numa branch. **Não** usar `helix-term` nem `helix-view`.

Isolar o uso dessas crates atrás de módulos nossos (principalmente em
`nas-editor`) para que uma atualização de tag quebre poucos lugares — essas
crates não expõem uma API estável/versionada por semver, só por tag de
release do editor.

## Prova (spike, etapa 0)

Protótipo em `spikes/helix-core/` (fora do workspace principal):

- Tag usada: **`25.07.1`** — a tag de release mais recente no momento da
  escrita (confirmada via `git ls-remote --tags` sobre o repositório oficial;
  não há tags `26.*` ainda).
- `cargo build` compilou de primeira, sem conflito de dependências e sem
  patches.
- Fluxo provado: abrir um arquivo num `Rope` (`Rope::from_reader`), aplicar
  uma `Transaction::insert` e uma `Transaction::delete`, desfazer as duas com
  `History::undo()`, imprimindo o buffer a cada passo.

Partes da API que exigiram ler o código-fonte da tag fixada (a
`helix-core` não tem documentação extensa nem exemplos de uso fora do
`helix-term`):

- `History::commit_revision(transaction, original)` recebe a transação **e**
  o `State` (rope + seleção) **anterior** à aplicação da transação — é ele
  quem calcula a inversão internamente, via `transaction.invert(&original.doc)`.
  Não dá para comitar depois de já ter mutado o rope sem guardar o estado
  anterior.
- `History::undo()` e `History::redo()` **retornam** a transação a aplicar,
  mas não a aplicam — quem chama precisa aplicar no rope manualmente.
- `Transaction::delete` recebe um iterador de pares `(from, to)` em
  **caracteres**, não bytes — importante para texto com acentos e caracteres
  multi-byte.

Nenhuma dificuldade de peso além de ler esse código; a API é pequena e
coerente.

## Alternativa descartada

Usar só `ropey` com um undo/redo escrito por nós, trazendo o `helix-core`
depois. Descartada porque o spike não revelou nenhum obstáculo real — a API
do `helix-core` é pequena, isolável por módulo, e já entrega rope, transações
e histórico prontos e testados pelo próprio projeto Helix, exatamente o que
os pilares do nas (mudanças da IA como transações reversíveis) exigem.
Reescrever isso por conta própria seria trabalho redundante sem ganho claro.

## Consequências

- Atualizar a tag do Helix é uma decisão deliberada, não automática — exige
  reler o changelog e, se necessário, ajustar os módulos de isolamento em
  `nas-editor`.
- Qualquer dúvida sobre a API dessas crates deve ser resolvida lendo o
  código-fonte da tag fixada (via `~/.cargo/git/checkouts/...` depois de um
  `cargo build`), não por suposição.
