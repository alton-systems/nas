# 0003 — nas-keys como keymap, e como mantê-lo em sincronia

## Status

Aceita; o mecanismo de sincronia descrito aqui é para o marco 1 e será
revisto no marco 2.

## Contexto

O sistema de teclas do nas é o [nas-keys](https://github.com/2gab/nas-keys),
um pacote de Emacs mantido ativamente pelo mesmo autor do nas
(`nas.el`, `nas-state.el`, `nas-map.el`, `nas-layout.el`, `nas-tests.el`). O
nas-keys vai continuar mudando de forma independente do nas. Se os atalhos,
estados e despachos forem codificados diretamente em Rust, toda mudança no
nas-keys vira um trabalho de reescrita de código em vez de uma edição de
dados — e os dois projetos divergem silenciosamente.

## Decisão

1. **O repositório nas-keys é a fonte da verdade.** Nenhuma decisão sobre
   estados, atalhos, prefixos ou despachos é tomada a partir de memória ou de
   um resumo — sempre clonar e ler a versão atual de `nas.el`, `nas-state.el`,
   `nas-map.el` e `nas-layout.el` antes de implementar ou alterar o keymap do
   nas.
2. **Estados, atalhos, prefixos e despachos vivem como dados declarativos**,
   não espalhados no código Rust: um arquivo único em
   `crates/nas-keys/keymap/default.toml` (etapa 4), espelhando a estrutura do
   `nas-map.el` — mesmos estados (`Insert`, `Visual`, `Sequential`), mesmos
   prefixos (`g`, Espaço), mesmos despachos de uma tecla (`d …`) e a mesma
   semântica do estado Sequential (acumula teclas, mostra na barra de status,
   executa com Enter, cancela com Esc, remove a última tecla com Backspace).
3. **Tradução de layout físico como tabela de dados**, não como `match` no
   código: tabelas `qwerty` e `qwerty-abnt` (o autor usa ABNT2), permitindo
   adicionar outros layouts sem tocar na lógica da máquina de estados.
4. **Portar os testes do `nas-tests.el`** que fizerem sentido em Rust, para
   que a máquina de estados do `nas-keys` (crate) tenha a mesma cobertura de
   comportamento que o pacote Elisp original.

## Mecanismo de sincronia (marco 1)

Por ora a sincronia é **manual e unidirecional**: quando o nas-keys (Emacs)
muda, alguém lê o diff do repositório nas-keys e replica a mudança de
semântica no arquivo de dados do nas (Rust) e nos testes portados. Não há
compartilhamento de arquivo entre os dois projetos ainda.

Isso é deliberadamente provisório. Há uma tarefa separada, no próprio
repositório nas-keys, para propor um formato de arquivo TOML comum (estados,
atalhos por posição qwerty, prefixos e despachos) que possa ser lido tanto
pelo pacote Elisp quanto pelo editor nas — a proposta deve ser apresentada
antes de qualquer mudança de código nos dois lados, para que os dois projetos
nunca divirjam de fato.

## Consequências

- Até a etapa 4, o keymap pode viver como estrutura de dados em memória (Rust)
  enquanto a máquina de estados é desenvolvida e testada (etapa 3); o arquivo
  `default.toml` é introduzido na etapa 4 como a representação canônica.
- A partir do marco 2, quando o Lisp embutido (Steel) estiver disponível, o
  keymap deve migrar para ser definido em Lisp — como no nas-keys é feito em
  Elisp — reduzindo ainda mais a distância entre os dois projetos. Essa
  migração será registrada num ADR próprio quando chegar a hora.
- Até que o formato comum proposto na tarefa do nas-keys seja adotado dos
  dois lados, a sincronia continua manual e depende de releitura periódica do
  repositório nas-keys.
