# 0008 — modo de revisão

## Status

Proposta — aguardando aprovação. Nada implementado; nenhum PR de
código abre a partir desta ADR até ela ser aceita (CLAUDE.md: parar
antes de uma decisão grande não registrada — e esta introduz o
primeiro estado de teclas e a primeira persistência de revisão do
nas, então fica registrada antes de qualquer código).

## Contexto

A ADR 0006 já esboça esta etapa (§6 e §8) e a nomeia simplesmente
"revisão", encaixada no roteiro revisado entre **3e** (autoria de
sessão, pronta) e **3f-prep** (eficiência das ferramentas do MCP):

> uma tela pra revisar o trabalho de uma sessão de IA (ou de um
> período) antes de confiar nele, trecho a trecho [...] vale nos dois
> desfechos do benchmark (3f) [...] também dá uma métrica de controle
> que o §7 ainda não media.

Esta ADR formaliza esse esboço em decisão de verdade — o que é
revisado, a tela, as ações, as teclas e a métrica — porque nenhuma
das cinco partes é trivial o bastante pra decidir só no código: a
tela introduz o **primeiro buffer somente leitura** do nas, as teclas
introduzem o **primeiro modo com atalhos próprios** (`docs/keys.md`
ainda só conhece três estados — Insert, Visual, Sequential, §2 — e a
paleta de comandos hoje lista `[commands]` sem olhar pra qual estado
está ativo, §5.9), e a persistência de "o que já foi revisado"
introduz um arquivo novo em `~/.local/state/nas/`.

**O que já existe e esta ADR reaproveita, sem redesenhar:**

- **Checkpoint automático de sessão** (ADR 0006 §6, etapa 3e):
  `crates/nas-commands/src/registry.rs::prepare_session_checkpoint`
  cria, na primeira chamada efetiva de um `(agent, session)`, um
  checkpoint automático `"antes da sessão de <agent> (<data>)"` com o
  conteúdo de todo arquivo rastreado — buffer aberto, se houver;
  senão o disco (mesma função `ctx.workspace.current_content_for`).
  Esse é o "antes" padrão desta ADR.
- **Diff entre dois checkpoints** (ADR 0005 §7):
  `nas_history::checkpoint::diff(history_dir, from_id, to_id)` compara
  dois `CheckpointManifest` já gravados, arquivo por arquivo, via
  `similar::TextDiff::from_lines(...).unified_diff()` — a mesma função
  que `Ctrl+t e` já usa (`nas-term/src/app.rs::checkpoint_diff`). Ela
  só sabe comparar **dois checkpoints que já existem em disco**; não
  compara um checkpoint contra o estado vivo (buffer/disco agora).
  Isso importa pro §2: o "depois" desta ADR é sempre "agora", nunca um
  segundo checkpoint — essa é precisamente a lacuna que a função de
  hoje não cobre, e que esta ADR pede pra preencher (§2).
- **Journal com autoria** (ADR 0005 §1/§3): cada
  `JournalEntry { author, command, file, from, to, removed, inserted }`
  guarda o suficiente pra saber quem tocou que faixa de caracteres e
  quando — e, por ADR 0005 §3, "reconstruir qualquer estado
  intermediário, aplicando ou invertendo entradas em sequência". Esta
  ADR é a primeira a precisar de verdade dessa reconstrução (§2,
  "misto").
- **`edit` por texto exato, nunca por deslocamento** (ADR 0006 §3): o
  comando de edição da IA busca o texto antigo exato e recusa se ele
  já não for mais o que está lá. A ação "rejeitar" desta ADR (§3)
  usa exatamente a mesma semântica, pelo mesmo motivo.
- **Issue #9** (modos: contexto + capacidades): propõe, pra depois da
  fase 3, um mecanismo genérico de "capacidades" declaradas como
  dados, com contexto, contribuição única e resolução de conflito.
  Esta ADR **não implementa** esse mecanismo — ele não existe ainda e
  a própria issue diz que não bloqueia a fase 3. O que esta ADR faz é
  construir o modo de revisão como um **quarto estado dedicado** da
  máquina de teclas (ao lado de Insert/Visual/Sequential), da forma
  mais simples que já respeita as regras que a issue #9 exige de
  qualquer modo futuro (um radical, um significado; só usa teclas
  ainda livres) — servindo, como o próprio pedido desta tarefa
  nomeia, de **modelo concreto** pro que a issue #9 vai generalizar
  depois. §4 detalha e volta a essa relação.

## Decisão

### 1. O que é revisado

**Padrão**: a sessão de IA mais recente — o checkpoint automático mais
recente cujo `author` é `Author::Ai { .. }` (`c.automatic &&
matches!(c.author, Author::Ai { .. })`, último da lista) é o
"**antes**". O "**depois**" é o estado atual de todo arquivo que o
checkpoint lista: o buffer aberto, se houver; senão o disco — a mesma
regra que `prepare_session_checkpoint` já usa pra montar o conteúdo
do próprio checkpoint, agora do lado do "depois" em vez do "antes".

**Escolher outro ponto de partida**: ao abrir a revisão (§4), uma
lista selecionável mostra os mesmos checkpoints que `Ctrl+t l` já
lista (nome, quando, autor, quantos arquivos) — não uma listagem
nova, os mesmos dados de `nas_history::list_checkpoints`. O checkpoint
padrão (acima) vem pré-selecionado; mover a seleção e confirmar troca
o "antes" por qualquer outro checkpoint do projeto, nomeado ou
automático, de qualquer autor — revisar não é exclusivo de sessão de
IA, só o é por padrão. **O "depois" é sempre "agora"**, nunca um
segundo checkpoint escolhido — comparar dois checkpoints fixos **já**
existe (`Ctrl+t e`); esta ADR não duplica isso, só cobre o caso que
falta (um checkpoint contra o estado vivo).

**Base = último commit do Git**: registrado aqui como opção futura,
fora desta ADR (§6). Pediria uma forma de ler o conteúdo de um blob do
Git sem rodar `git` (ADR 0005 §6: "isso só lê a presença do `.git` [...]
nunca escreve nem roda nenhum comando `git`") — um mecanismo novo que
não existe hoje e que vale isolar numa decisão própria quando for
implementado, não misturado a esta.

### 2. A tela

**Um buffer somente leitura de tipo novo.** `nas-term::Buffer` ganha
um campo `read_only: bool` (falso pra todo buffer de hoje). O
conteúdo é um `Document` **sintético**, gerado na hora de abrir a
revisão — nunca lido nem escrito em disco, nunca `is_modified()`,
nunca aparece em `Ctrl+b` como "arquivo do projeto" (é removido da
lista de buffers ao sair da revisão, §4). `read_only` bloqueia, nesse
buffer, exatamente dois atalhos da classe sistema que não fariam
sentido nele: `Ctrl+s` (salvar) e `Ctrl+r` (recarregar do disco) —
ambos respondem na echo area ("buffer somente leitura"), em vez de
tentar agir sobre um caminho que não existe. Os demais (`Ctrl+b`,
`Ctrl+k`, `Ctrl+q`) continuam funcionando normalmente — fechar ou
trocar de buffer de uma revisão não precisa de tratamento especial,
porque ela nunca tem alteração não salva pra perguntar sobre.

**Conteúdo do buffer, de cima pra baixo:**

1. **Resumo**: quantos arquivos, quantos trechos, quantos já
   revisados (aceitos + rejeitados) — texto simples, as primeiras
   linhas do buffer, sem nada de interativo próprio (é só texto; `g g`
   volta pra lá como pra qualquer início de buffer).
2. **Um bloco por arquivo tocado**, cada um com **um trecho por
   região alterada**, em diff unificado (linhas `-`/`+`, coloridas —
   vermelho/verde, o mesmo par que o resto do nas já reserva pra
   remover/adicionar, se houver; senão duas cores distintas simples),
   com um cabeçalho de trecho: **arquivo, linha (no "depois"), autor,
   sessão** — ex. `src/foo.rs:42  IA (claude-code, sessão a1b2)`.

**Diff: nova função em `nas-history`, reaproveitando o cálculo, não
reimplementando.** `checkpoint::diff` (ADR 0005 §7) já faz exatamente
o que se precisa — `similar::TextDiff::from_lines(...).unified_diff()`
por arquivo — só que lendo os dois lados de `objects/` via dois
`CheckpointManifest`. Esta ADR pede uma função nova e pequena,
`diff_against_current(history_dir, checkpoint_id, current: &[(path,
content)]) -> io::Result<Vec<FileDiff>>`, que faz a mesma coisa
trocando o lado "to" por conteúdo já em memória (vindo do buffer ou
do disco, nunca de um checkpoint de verdade) em vez de um segundo
manifesto — o `unified_text`/comparação de hash que `diff_file` já
faz internamente não muda; só o lado de onde vem o "depois" muda. O
tipo de retorno (`FileDiff`, `FileChange`) é o mesmo que `Ctrl+t e` já
usa — a tela de revisão não introduz um formato de diff paralelo.

Cada "trecho" exibido é um **hunk** do diff unificado — o próprio
formato que `similar` já produz delimita onde um hunk começa e
termina; a tela de revisão não precisa reagrupar linhas por conta
própria, só iterar sobre os hunks que o unified diff já teria entre
dois `@@ ... @@`.

**Autoria de um trecho, e "misto".** O cabeçalho de cada trecho mostra
a autoria do journal (ADR 0005 §1), não um dado novo — mas "qual
autoria" não é simplesmente "o autor do checkpoint": o trecho pode
ter sido tocado por mais de um autor desde o checkpoint (a IA edita,
e o usuário edita por cima, ou vice-versa). A regra:

- Toda `JournalEntry` do arquivo com `ts_start` depois da criação do
  checkpoint ("antes") é uma candidata.
- Como os `from`/`to` de cada entrada são offsets de **quando ela foi
  gravada**, não remapeados — e o texto já mudou desde então, por
  definição, se há mais de uma entrada — cada entrada precisa ser
  **remapeada pra coordenadas do texto atual** antes de comparar com
  a posição do trecho: compor, em ordem cronológica, o `ChangeSet`
  equivalente a cada entrada (derivável de `removed`/`inserted` —
  exatamente a reconstrução que a ADR 0005 §3 já promete ser possível)
  e aplicar `map_pos` de cada entrada posterior sobre a posição da
  anterior. **É o mesmo mecanismo que a ADR 0006 §2.1 já decide** pra
  remapear o cursor entre transações de autores diferentes — só que
  ali é ao vivo (3b, ainda não implementado) e aqui é offline, sobre
  entradas já no journal. Esta ADR não inventa um segundo algoritmo de
  remapeamento; usa o mesmo, só que replay em vez de ao vivo.
- Se, depois do remapeamento, mais de um autor distinto tem entrada
  cujo intervalo intersecta a faixa do trecho no texto atual, o
  cabeçalho mostra **"misto"** em vez de um autor só (ex. `src/foo.rs:
  42  misto (IA (claude-code) + você)`).
- Se não há **nenhuma** entrada do journal cobrindo aquele trecho
  (caso raro — ex. histórico podado, ADR 0005 §5), o cabeçalho mostra
  "autoria desconhecida" em vez de inventar um autor.

Isso é trabalho novo em `nas-history` (algo como
`attribute_hunk(history_dir, file, since_ts, range_in_current_text) ->
Vec<Author>`), mas é composição de três coisas que já existem
(journal, `ChangeSet`/`map_pos` do helix-core, e a decisão já tomada
de que isso é possível) — não um mecanismo de autoria novo.

### 3. Ações por trecho

Três ações, cada uma agindo sobre o trecho sob o cursor (ou
selecionado, §4):

- **Aceitar**: marca o trecho como revisado. **Persiste** — reabrir a
  mesma revisão (mesmo checkpoint "antes") depois mostra que aquele
  trecho já foi visto, mesmo que o nas tenha sido fechado e reaberto
  entre as duas vezes. Não muda nenhum texto — é só um registro (§5).
- **Rejeitar**: desfaz **só aquele trecho** no arquivo — restaura, só
  naquela faixa, o texto do lado "antes" do trecho (o conteúdo do
  checkpoint) — como uma edição comum do usuário: passa por
  `Document::apply`, autor `Author::User`, comando
  `"review-reject"`, journalada e desfazível com `u`, exatamente como
  qualquer outra tecla do usuário. **Só funciona se o texto atual
  daquele trecho ainda for exatamente o que a IA deixou** — a mesma
  condição que `edit` já impõe antes de aplicar (ADR 0006 §3): se
  outra edição já tocou aquele texto desde então (um `aceitar`
  seguinte da IA, ou uma edição manual sobreposta), rejeitar recusa
  com uma mensagem clara em vez de aplicar sobre o lugar errado —
  nunca adivinha. Depois de rejeitar com sucesso, o trecho marca como
  revisado também (rejeitar é uma forma de revisar, não um terceiro
  estado "pendente").
- **Ir até lá**: abre o arquivo (buffer já aberto, ou abre um novo)
  na linha do trecho (lado "depois") e **sai da revisão** — não há
  "voltar pra revisão" automático; reabrir é `Ctrl+t v` de novo, que
  retoma de onde a persistência do `aceitar`/`rejeitar` deixou.

Um trecho sem seleção/cursor sobre ele (cursor no resumo, por
exemplo) não tem uma ação válida — as três teclas abaixo não fazem
nada fora de um trecho, e a echo area avisa isso.

### 4. Teclas

**Este é o primeiro modo com teclas próprias do nas.** A decisão
central: a revisão é um **quarto `ModalState`** (`Review`, ao lado de
`Insert`/`Visual`/`Sequential`, docs/keys.md §2/§4.4) — não o
mecanismo genérico de "capacidades" que a issue #9 propõe (esse não
existe ainda, e a própria issue diz que não bloqueia a fase 3).
`Review` tem sua própria tabela em `default.toml`, `[review]`, nova
— **não** uma tabela derivada automaticamente de `[visual]` por
herança (`machine.rs` não ganha um mecanismo de "modo que estende
outro modo" ainda; isso fica pra quando a issue #9 chegar). Em vez
disso, `[review]` **declara de novo**, como dado duplicado e
deliberado, só o subconjunto de `[visual]` que ainda faz sentido num
buffer sem edição — movimento por caractere/linha/extremo (`j k l ;`,
`J : K L`) e os prefixos `g` (ir pra início do buffer, ir pra linha) e
`Espaço f` (buscar no texto do diff). **Fora** de `[review]`: os
verbos (`d c x`), seleção/colar (`s S y`), undo (`u`, não há nada pra
desfazer *no próprio buffer* — desfazer uma rejeição é `u` no arquivo
de destino, não aqui) e repetir (`.`). Essa duplicação é pequena (meia
dúzia de linhas de TOML) e evita inventar herança de tabela antes de
precisar — exatamente a regra do `CLAUDE.md` de não abstrair
antecipando o que ainda não existe.

| Teclas | Ação | Por que essa escolha, contra `docs/keys.md` |
|---|---|---|
| `Ctrl+t v` | abre a revisão (lista de checkpoints, §1) | `v` — "ver/revisar"; livre em `[system_prefixes.tempo]` (hoje usa `n l e r a m z`) |
| `h k` / `h l` | próximo / trecho anterior | `h` como **unidade** nova ("trecho"), seguindo exatamente o padrão de `w`/`W` (unidade + direção, §5.1 de `docs/keys.md`): a direção vem das mesmas teclas que já significam "abaixo"/"acima" (`k`/`l`), igual a `W k`/`W l` pra parágrafo — regra 1 (`k`/`l` continuam significando a mesma coisa), nenhum radical de movimento é redefinido |
| `H k` / `H l` | próximo / arquivo anterior | `H` (Shift de `h`) — regra 3 do `docs/keys.md`: Shift intensifica a **escala** do mesmo radical, do mesmo jeito que `w`→`W` escala palavra→parágrafo; aqui trecho→arquivo é a escala natural seguinte |
| `Enter` | aceitar o trecho sob o cursor | reaproveita o significado que `Enter` já tem em **todo** outro estado — "avançar/confirmar o que está pendente" (`Visual`: entra no Insert; `Sequential`: executa a frase) — nunca uma redefinição isolada só pra este modo |
| `r` | rejeitar o trecho sob o cursor | letra livre em `[visual]` hoje (não ligada a nada, só `Ctrl+r` existe, classe diferente — mesmo raciocínio que a ADR 0005 §8 já usa pra `Ctrl+t a` vs `Ctrl+b`: modificadores diferentes reduzem o risco de confundir, por isso o nas já aceita esse tipo de reuso entre classes) |
| `v` | ir até lá (abre o arquivo na linha) | letra livre; eco deliberado do `v` de `Ctrl+t v` (mesma ideia — "ver" —, classes diferentes, mesmo raciocínio da linha acima) |
| `Esc` | sai da revisão, volta pro Visual | reaproveita o significado que `Esc` já tem em qualquer estado: cancelar a camada atual — aqui, a camada é a revisão inteira |

`Ctrl+t v`, como qualquer entrada no prefixo "tempo", lista o resultado
de `nas_history::list_checkpoints` numa seleção — a **mesma** listagem
de `Ctrl+t l`, agora navegável com `↑`/`↓` (como a paleta já faz,
§5.9) em vez de só mostrada (como o overlay hoje usado por `Ctrl+t
l`/`a`/`m`, que fecha em qualquer tecla). Isso é a primeira lista
selecionável fora da paleta de comandos — reaproveita o mesmo widget
de seleção (`↑`/`↓` dando a volta nas pontas, `Enter` confirma, `Esc`
cancela), não inventa um terceiro jeito de escolher de uma lista.

**Echo area e paleta.** A dica which-key (`docs/keys.md` regra 8), ao
entrar em `Review`, mostra as teclas de `[review]` do mesmo jeito que
já mostra as de qualquer prefixo/verbo pendente — a diferença é que
aqui a dica fica **persistente** enquanto o estado for `Review` (não
só até a tecla seguinte), igual a como a barra de status já mostra o
estado modal ativo (`[Visual]`, `[Insert]`) — passa a mostrar
`[Revisão]`.

A paleta de comandos (§5.9 de `docs/keys.md`) hoje lista **todo**
`[commands]` do keymap, sem olhar o estado ativo — nunca precisou,
porque só `Visual` tinha comandos "soltos" de verdade (Insert e
Sequential não usam a paleta). Com `[review]` como uma quarta tabela
de comandos reais (aceitar, rejeitar, ir até lá, navegar por
trecho/arquivo), listar esses comandos na paleta **fora** da revisão
seria mostrar ações sem nenhum trecho pra agir — por isso **o
catálogo da paleta passa a filtrar por `ModalState` atual**
(`Machine::command_catalog` ganha o estado como parâmetro, ou um
catálogo por estado em vez de um só global) — uma extensão pequena e
necessária, não uma rearquitetura: antes desta ADR, filtrar não fazia
diferença nenhuma (só havia uma tabela de comandos "soltos"); agora
há duas.

**Relação com a issue #9.** A issue propõe, pra depois da fase 3, um
mecanismo genérico onde "um modo pode acrescentar teclas novas, mas
nunca pode mudar o significado de um radical que já existe" e "só
podem usar teclas ainda não ligadas a nada". `Review`, como
implementado aqui, **já obedece as duas regras** — só que à mão
(tabela duplicada, estado dedicado) em vez de através do mecanismo de
capacidades (contexto + contribuição única + resolução de conflito,
ainda não construído). Esta ADR não implementa a generalização: é o
**primeiro caso concreto** que a issue #9 pode usar como teste,
quando chegar a hora de generalizar — "um modo principal cuja
condição de contexto é 'o buffer atual é uma revisão'" é, na linguagem
da própria issue, exatamente um "modo principal" (major-mode). Nada
nesta ADR impede essa migração futura; `[review]` como tabela
separada é dado, não código, então mover essa tabela pra dentro de um
manifesto de capacidade (quando existir) é uma mudança de onde o dado
vive, não do que ele diz.

### 5. Métrica para o benchmark

Duas coisas novas em `~/.local/state/nas/history/<hash>/` (mesma
pasta por projeto que journal/checkpoints/permissões já usam, ADR
0005 §5):

- **`review_marks.jsonl`** — append-only, uma linha por decisão
  (aceitar ou rejeitar), nunca podada automaticamente (é pequena,
  só texto, como `permissions_log.jsonl`):
  ```json
  {"ts": 1732000000, "reviewer": "você", "from_checkpoint": "<id>", "file": "crates/foo/src/lib.rs", "hunk_hash": "<sha256 do texto 'antes' do trecho>", "decision": "accepted"}
  ```
  `hunk_hash` identifica o trecho por um hash do seu conteúdo **do
  lado "antes"** (mais o caminho e o id do checkpoint) — estável
  entre reaberturas da mesma revisão mesmo que outras partes do
  arquivo mudem depois, porque o lado "antes" nunca muda (é um
  checkpoint já gravado). Reabrir a revisão recalcula os trechos do
  zero (§2) e cruza cada um contra as marcas já existentes pra esse
  `from_checkpoint`, mostrando o que já foi visto.
- **`reviews_log.jsonl`** — append-only, uma linha por sessão de
  revisão (não de IA — de uso do próprio modo de revisão), aberta ao
  entrar em `Review` e fechada ao sair (`Esc`, trocar de buffer, ou
  fechar o nas com a revisão aberta):
  ```json
  {"started_at": 1732000000, "ended_at": 1732000120, "from_checkpoint": "<id>", "total_hunks": 7, "accepted": 5, "rejected": 1}
  ```
  (2 em 7 pendentes se a revisão for interrompida antes de terminar —
  `accepted + rejected` pode ser menor que `total_hunks`, e isso é
  esperado, não um erro.)

Essa é a medida de **controle** que a ADR 0006 §7 ainda não tinha:
"quanto trabalho dá pra revisar a mudança [...] medido em
teclas/comandos" — com `reviews_log.jsonl`, o tempo de parede
(`ended_at - started_at`) e a proporção aceito/rejeitado por sessão
entram direto na tabela de integridade/rastreabilidade do benchmark
(§7, quadro separado — "não é uma comparação simétrica", o lado sem
nas não tem equivalente), sem precisar de nenhuma contagem manual.

### 6. Fora de escopo

- **Lado a lado** — depende de janelas (ADR 0006 §8, "fora da fase 3,
  só depois da decisão"); o diff unificado (§2) já não depende disso.
- **Desfazer seletivo de um trecho que o usuário editou depois da
  IA** — já era fora de escopo da ADR 0006 (§2.1, §6: "desfazer
  seletivo por autor [...] ideia pra depois"). Rejeitar (§3) só desfaz
  um trecho que ainda é exatamente o que a IA deixou — não tenta
  reconciliar uma edição do usuário por cima.
- **Revisão pela IA via MCP** — esta ADR é sobre a tela que uma pessoa
  usa; expor "aceitar"/"rejeitar"/"listar trechos pendentes" como
  ferramentas MCP pra outro agente revisar é uma extensão possível,
  não decidida aqui.
- **Comparar contra o último commit do Git como base** — §1, acima.

## Arquitetura

- **`nas-history`**: `diff_against_current` (§2), `attribute_hunk`
  (§2), `review_marks`/`reviews_log` — leitura e escrita (§5). Nenhuma
  dependência nova; `similar` e o próprio `helix_core::ChangeSet` (já
  usado indiretamente via `nas-editor`) bastam.
- **`nas-keys`**: `ModalState::Review`, tabela `[review]` em
  `default.toml`, a unidade nova "trecho" (`h`/`H`) ao lado de
  "word"/"paragraph" em `machine.rs` — genérica (não hardcoded pra
  revisão: o mecanismo de unidade já existe, só ganha mais um par de
  nomes possíveis) — e `command_catalog` filtrado por estado.
- **`nas-term`**: `Buffer::read_only`, a geração do `Document`
  sintético da revisão, a renderização do diff colorido, o widget de
  seleção reaproveitado pra listar checkpoints (§4), e o despacho das
  três ações (§3) chamando `Document::apply` (rejeitar) ou só
  gravando em `nas-history` (aceitar).
- **`nas-commands`**: nenhuma mudança — revisão é uma funcionalidade
  do `nas-term` sobre o histórico, não um comando do registro (§6,
  "fora de escopo": nenhuma ferramenta MCP nova).
- **`docs/keys.md`**: ganha uma seção `5.10` documentando `[review]`
  e o estado `Review`, e a entrada de `Ctrl+t v` na tabela do prefixo
  "tempo" (§5.7) — parte da implementação, não desta ADR.

## Critério de pronto

1. Com uma sessão de IA simulada (`Author::Ai`, como a ADR 0006 §6 já
   testa) tendo editado dois arquivos, `Ctrl+t v` sem escolher nada
   abre a revisão no checkpoint automático daquela sessão contra o
   estado atual — mostrando os trechos certos, com autoria correta.
2. Editar manualmente, por cima da IA, uma faixa dentro de um dos
   trechos (sem passar pelo `aceitar`) faz aquele trecho aparecer como
   "misto" na reabertura da revisão.
3. Aceitar um trecho, fechar o nas, reabrir a mesma revisão: o trecho
   aparece marcado como já revisado.
4. Rejeitar um trecho cujo texto ainda é o que a IA deixou desfaz só
   aquele trecho, journalado como `Author::User`, desfazível com `u`;
   rejeitar um trecho já alterado por outra edição recusa com uma
   mensagem, sem tocar no arquivo.
5. `h`/`H` navegam entre trechos e arquivos sem jamais mudar o
   significado de `j k l ; J : K L` em `Review` comparado a `Visual`.
6. `reviews_log.jsonl` registra início, fim e as contagens de uma
   sessão de revisão completa.

## Consequências

- Primeiro `ModalState` novo desde o Marco 1 — `docs/keys.md` §2/§4.4
  precisa de uma atualização (parte da implementação).
- Primeira vez que a paleta de comandos depende do estado ativo —
  mudança pequena, mas toca `Machine::command_catalog`, usado hoje sem
  esse parâmetro.
- Primeiro uso real da promessa da ADR 0005 §3 ("reconstruir qualquer
  estado intermediário [...] aplicando ou invertendo entradas em
  sequência") e do mecanismo de remapeamento da ADR 0006 §2.1
  (`ChangeSet`/`map_pos`), replay em vez de ao vivo — nenhuma das duas
  precisa de redesenho, só de uma consumidora de verdade.
- `~/.local/state/nas/history/<hash>/` ganha dois arquivos
  (`review_marks.jsonl`, `reviews_log.jsonl`), mesma pasta, mesmo
  padrão append-only já usado por `journal.jsonl`/`permissions_log.jsonl`.
- Prepara terreno pra issue #9 sem implementá-la: `Review` é o
  primeiro "modo" de verdade do nas, construído da forma mais simples
  possível hoje (estado dedicado, tabela duplicada) — candidato
  natural a migrar pro mecanismo de capacidades quando ele existir,
  sem precisar mudar o que o modo *faz*, só onde ele é declarado.
