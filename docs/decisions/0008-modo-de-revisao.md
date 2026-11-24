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
  dados, com contexto, contribuição única e resolução de conflito —
  nada disso existe ainda, e a própria issue diz que não bloqueia a
  fase 3. Esta ADR não implementa esse mecanismo genérico, mas
  implementa o **primeiro passo concreto** na direção que ele pede:
  uma camada de teclas (`[review]`) consultada antes de `[visual]`,
  que só pode acrescentar teclas novas ou bloquear ações por nome —
  nunca redefinir o que uma tecla não bloqueada já significa, porque
  o que não está na camada simplesmente cai no keymap resolvido (com
  o `keymap.toml` do usuário já aplicado) de baixo. §4 detalha e volta
  a essa relação.

## Decisão

### 1. O que é revisado

**Padrão**: a sessão de IA mais recente — o checkpoint automático mais
recente cujo `author` é `Author::Ai { .. }` (`c.automatic &&
matches!(c.author, Author::Ai { .. })`, último da lista) é o
"**antes**". O "**depois**" é o estado atual de todo arquivo que o
checkpoint lista: o buffer aberto, se houver; senão o disco — a mesma
regra que `prepare_session_checkpoint` já usa pra montar o conteúdo
do próprio checkpoint, agora do lado do "depois" em vez do "antes".

**Dois jeitos de abrir, dois escopos** (§4 detalha as teclas):
`Ctrl+t v` abre a revisão **direto** no padrão acima — sem lista, sem
pergunta, pro caso comum (revisar o que a sessão de IA acabou de
fazer). `Ctrl+t V` (Shift) abre uma lista selecionável com os mesmos
checkpoints que `Ctrl+t l` já lista (nome, quando, autor, quantos
arquivos) — não uma listagem nova, os mesmos dados de
`nas_history::list_checkpoints` — com o checkpoint padrão
pré-selecionado; escolher outro troca o "antes" por qualquer
checkpoint do projeto, nomeado ou automático, de qualquer autor —
revisar não é exclusivo de sessão de IA, só o é por padrão. **O
"depois" é sempre "agora"**, nunca um segundo checkpoint escolhido —
comparar dois checkpoints fixos **já** existe (`Ctrl+t e`); esta ADR
não duplica isso, só cobre o caso que falta (um checkpoint contra o
estado vivo).

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
  estado "pendente") — e, como o texto volta a ser igual ao do
  checkpoint, o trecho deixa de aparecer na próxima vez que a
  revisão recalcular os trechos (§2): não há mais diferença ali pra
  mostrar. A tecla que aciona isto dentro da revisão é `u` (§4) — a
  mesma tecla que, no arquivo real, desfaz a última transação
  (inclusive, se for o caso, a que o próprio rejeitar acabou de
  criar): dois contextos, não dois significados.
- **Ir até lá**: abre o arquivo (buffer já aberto, ou abre um novo)
  na linha do trecho (lado "depois") e **sai da revisão** — não há
  "voltar pra revisão" automático; reabrir é `Ctrl+t v` (ou `Ctrl+t
  V`, se o checkpoint "antes" não for o padrão) de novo, que retoma
  de onde a persistência do `aceitar`/`rejeitar` deixou.

Um trecho sem seleção/cursor sobre ele (cursor no resumo, por
exemplo) não tem uma ação válida — as três teclas abaixo não fazem
nada fora de um trecho, e a echo area avisa isso.

### 4. Teclas

**Este é o primeiro modo com teclas próprias do nas.** `Review` ainda
é um **quarto `ModalState`** (ao lado de `Insert`/`Visual`/`Sequential`,
docs/keys.md §2/§4.4) — mas sua tabela, `[review]`, **não é uma
cópia** de `[visual]`. É uma **camada consultada primeiro**: só
declara o que é novo (as unidades de navegação, aceitar) ou precisa
ser **bloqueado por não fazer sentido num buffer somente leitura**
(verbos, seleção/colar, repetir). Qualquer tecla que `[review]` não
menciona cai no `[visual]` **já resolvido com o `keymap.toml` do
usuário aplicado** — nunca no `default.toml` cru. Isso importa de
verdade: se o usuário remapeou um movimento (trocou `j`/`k`/`l`/`;`
por outra coisa em `keymap.toml`, docs/config.md), essa troca precisa
valer dentro da revisão também, sem que `[review]` precise saber que
ela existe — uma tabela duplicada (o desenho de um rascunho anterior
desta ADR) quebraria justo esse caso, porque o movimento duplicado
ficaria sempre congelado no `default.toml`.

**O bloqueio é por nome de ação, não por tecla física.** `[review]`
não proíbe "a tecla `d`" — proíbe os **comandos** que não fazem
sentido num buffer somente leitura: qualquer verbo
(`delete`/`copy`/`cut`), `start-selection`, `select-all`, `paste` e
`repeat`. Pelo mesmo motivo do parágrafo acima: se o usuário remapeou
o verbo "apagar" pra uma tecla que não é `d`, o bloqueio ainda precisa
valer — é o nome do comando que está fora da revisão, não uma tecla
física específica. Uma tecla bloqueada avisa na echo area que aquela
ação não está disponível ali, nunca falha em silêncio.

`u` (desfazer) **não entra no bloqueio — é redefinido dentro da
camada**, de propósito, e não é uma tecla nova: `u` já significa
"desfazer"; rejeitar um trecho **é** desfazer aquela mudança
específica da IA, só que o alvo é "o trecho sob o cursor" em vez de
"a última transação de qualquer autor" (`Visual`). Não é um radical
ganhando um segundo significado — é o mesmo significado, com o alvo
que faz sentido numa tela que mostra trechos, não transações soltas.

| Teclas | Ação | Por que essa escolha, contra `docs/keys.md` |
|---|---|---|
| `Ctrl+t v` | abre a revisão **direto**, no padrão (§1) — sem lista | `v` — "ver/revisar"; livre em `[system_prefixes.tempo]` (hoje usa `n l e r a m z`) |
| `Ctrl+t V` | abre a revisão com a **lista** de checkpoints pra escolher (§1) | Shift aplicando a regra 3 de `docs/keys.md` fora de `[visual]`: leva a ação ao seu extremo — em vez do caminho direto, mostra **todas** as opções antes de decidir; primeiro uso de Shift dentro de um prefixo de sistema, mas a mesma ideia de escala |
| `h k` / `h l` | próximo / trecho anterior | `h` como **unidade** nova — "trecho alterado" — seguindo o padrão de `w`/`W` (unidade + direção, §5.1): a direção usa as mesmas teclas que já significam "abaixo"/"acima" (`k`/`l`), igual a `W k`/`W l` pra parágrafo; `k`/`l` continuam significando a mesma coisa (regra 1) |
| `H k` / `H l` | próximo / arquivo anterior | `H` (Shift de `h`) — regra 3: escala trecho → arquivo, o degrau seguinte depois de um trecho; só faz sentido com mais de um arquivo em jogo, por isso fica só em `[review]` (`h` generaliza, `H` não — ver nota abaixo) |
| `Enter` | aceitar o trecho sob o cursor | reaproveita o significado que `Enter` já tem em todo outro estado — avançar/confirmar o que está pendente (`Visual`: entra no Insert; `Sequential`: executa a frase) |
| `u` | rejeitar o trecho sob o cursor | **não é uma tecla nova** — é o `u` de sempre (desfazer), com o alvo "o trecho atual" em vez de "a última transação" (ver acima); por isso não entra no bloqueio nem precisa de uma letra livre |
| `g h` | ir até lá — abre o arquivo real na linha do trecho | **nem é uma entrada de `[review]`**: é uma entrada nova em `[prefixes.go]`, global (§5.5), compondo o prefixo que já significa "ir para" (`g`) com o radical novo "trecho" (`h`) — "ir para [o lugar d]o trecho atual". Fora da revisão, sem um trecho atual, a tecla não tem efeito — mesma ideia de tecla válida sem alvo que já existe noutros compostos |
| `Esc` | sai da revisão, volta pro Visual | a mesma cascata que `Esc` já segue hoje (cancela verbo/prefixo/unidade pendente; sem nada pendente, desfaz seleção; sem seleção, não faz nada) ganha mais um degrau no final, específico de `Review`: sem nada pendente e sem seleção, sai da revisão |

**`h` é um radical do nas inteiro — não só da revisão.** O
significado de `h` fica fixado agora como "trecho alterado", pra
qualquer buffer, não só dentro de `Review` — ainda que, hoje, só
`[review]` de fato ligue `h` a uma ação (limitação reconhecida no
critério de pronto, abaixo). `docs/keys.md` §6 (teclas reservadas)
ganha uma linha: `h k`/`h l`, em buffers comuns, reservado pra um
futuro "próxima/anterior mudança" (contra o checkpoint mais recente,
presumivelmente — não decidido aqui, só reservado, do mesmo jeito que
`p` já é reservado por outro motivo). **`H` não generaliza**: "o
próximo arquivo" só faz sentido com mais de um arquivo em jogo ao
mesmo tempo, o que só a revisão tem; fica só em `[review]`, sem
entrada em §6.

`Ctrl+t V`, ao abrir a lista, reaproveita o mesmo widget de seleção
que a paleta de comandos já usa (`↑`/`↓` dando a volta nas pontas,
`Enter` confirma, `Esc` cancela) — a primeira lista selecionável fora
da paleta, mas o mesmo mecanismo, não um terceiro jeito de escolher.

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
de comandos reais (aceitar, navegar por trecho/arquivo — `u`
continua sendo "desfazer", não um nome de comando novo), listar esses
comandos na paleta **fora** da revisão seria mostrar ações sem nenhum
trecho pra agir — por isso **o catálogo da paleta passa a filtrar por
`ModalState` atual** (`Machine::command_catalog` ganha o estado como
parâmetro, ou um catálogo por estado em vez de um só global) — uma
extensão pequena e necessária, não uma rearquitetura: antes desta
ADR, filtrar não fazia diferença nenhuma (só havia uma tabela de
comandos "soltos"); agora há duas. `g h` (ir até lá), por viver em
`[prefixes.go]` — global, não em `[review]` — continua aparecendo na
paleta em qualquer estado, como o resto do prefixo "ir para"; fora da
revisão ela só não tem efeito (mesma ideia de tecla válida sem alvo
já registrada na tabela acima).

**Relação com a issue #9.** A issue propõe, pra depois da fase 3, um
mecanismo genérico de camadas: "um modo pode acrescentar teclas
novas, mas nunca pode mudar o significado de um radical que já
existe" e "só podem usar teclas ainda não ligadas a nada". O desenho
de `[review]` acima — camada consultada primeiro, bloqueio por nome
de ação, o resto cai no keymap resolvido — **já é**, em miniatura,
esse mecanismo: não uma analogia que a issue #9 precisaria
reconstruir do zero depois, mas o **primeiro código real** de "uma
camada de teclas sobre outra" que existe no nas. O que esta ADR
**não** faz é generalizar: `[review]` continua sendo a única camada
possível (não há como declarar uma segunda, nem compor duas), o
bloqueio por nome de ação é uma lista fixa em Rust (não um dado de
manifesto), e não há resolução de conflito nenhuma, porque só existe
uma camada pra conflitar com `[visual]`. Isso tudo fica pra quando a
issue #9 generalizar — trocando esta camada única por N camadas
declaradas como dados, com contexto e resolução de conflito — sem
precisar mudar o que `Review` *faz*, só como ela está montada.

### 5. Métrica para o benchmark

Duas coisas novas em `~/.local/state/nas/history/<hash>/` (mesma
pasta por projeto que journal/checkpoints/permissões já usam, ADR
0005 §5):

- **`review_marks.jsonl`** — append-only, uma linha por decisão
  (aceitar ou rejeitar), nunca podada automaticamente (é pequena,
  só texto, como `permissions_log.jsonl`):
  ```json
  {"ts": 1732000000, "reviewer": {"kind": "user"}, "from_checkpoint": "<id>", "file": "crates/foo/src/lib.rs", "hunk_hash": "<sha256 de caminho + checkpoint + texto 'antes' + texto 'depois' + linha de início no 'antes'>", "decision": "accepted"}
  ```
  `reviewer` é a **mesma serialização de `Author`** que o journal e os
  checkpoints já usam (`Author::User` aqui, igual ao que `ADR 0005
  §1` já define) — não um texto livre como "você"; a revisão já sabe
  o autor de quem está revisando (o `Author` corrente do processo), e
  inventar uma segunda representação só pra esta marca não ganharia
  nada.

  `hunk_hash` **não pode ser só o texto "antes"**: um trecho que
  **só insere** linhas (comum — a IA acrescentando uma função nova)
  tem o lado "antes" vazio, e **duas inserções diferentes no mesmo
  arquivo colidiriam** no mesmo hash — aceitar uma marcaria a outra
  como revisada por engano. O hash cobre cinco coisas: caminho do
  arquivo, id do checkpoint ("antes"), o texto do lado "antes", o
  texto do lado "depois" e a linha de início do trecho no "antes" (o
  que desempata duas inserções vazias que também inserissem
  exatamente o mesmo texto em dois lugares — a posição, aí, ainda
  diferencia). Isso continua estável entre reaberturas da **mesma**
  revisão (mesmo `from_checkpoint`): o lado "antes" nunca muda (é um
  checkpoint já gravado), e a linha de início de um trecho específico
  no "antes" também não, mesmo que outros trechos do arquivo sejam
  aceitos ou rejeitados entre uma abertura e outra — cada trecho é
  identificado pela própria posição no "antes", nunca pela posição
  relativa aos outros. Reabrir a revisão recalcula os trechos do zero
  (§2) e cruza cada um contra as marcas já existentes pra esse
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
- **`nas-keys`**: `ModalState::Review`; a resolução de tecla em
  `Review` passa a ser "consulta `[review]`, senão cai no `[visual]`
  já resolvido" — um mecanismo genérico de camada (uma tabela sobre
  outra), não específico de revisão, reaproveitável pela issue #9
  depois — com um bloqueio por **nome de ação** (verbos,
  `start-selection`, `select-all`, `paste`, `repeat`), não por tecla
  física, pra valer sobre um keymap remapeado pelo usuário. A unidade
  "trecho" (`h`) e, só dentro de `[review]`, "arquivo" (`H`) entram
  no mesmo mecanismo genérico de unidade que já existe
  (`word`/`paragraph`) — mais dois nomes possíveis, não lógica nova.
  `[prefixes.go]` (global) ganha a entrada `h` ("ir até o trecho
  atual" — sem efeito fora da revisão). `command_catalog` filtrado
  por estado, como antes.
- **`nas-term`**: `Buffer::read_only`, a geração do `Document`
  sintético da revisão, a renderização do diff colorido, o widget de
  seleção reaproveitado pra listar checkpoints (§4), e o despacho das
  três ações (§3) chamando `Document::apply` (rejeitar) ou só
  gravando em `nas-history` (aceitar).
- **`nas-commands`**: nenhuma mudança — revisão é uma funcionalidade
  do `nas-term` sobre o histórico, não um comando do registro (§6,
  "fora de escopo": nenhuma ferramenta MCP nova).
- **`docs/keys.md`**: ganha uma seção `5.10` documentando a camada
  `[review]` e o estado `Review` (o mecanismo "consulta primeiro,
  senão cai no de baixo" e o bloqueio por nome de ação); `Ctrl+t v` e
  `Ctrl+t V` na tabela do prefixo "tempo" (§5.7 — primeiro uso de
  Shift dentro de um prefixo de sistema); `g h` em `[prefixes.go]`
  (§5.5); e uma linha nova em §6 (teclas reservadas) pra `h k`/`h l`
  fora da revisão. Tudo parte da implementação, não desta ADR.

## Critério de pronto

1. Com uma sessão de IA simulada (`Author::Ai`, como a ADR 0006 §6 já
   testa) tendo editado dois arquivos, `Ctrl+t v` abre **direto** a
   revisão no checkpoint automático daquela sessão contra o estado
   atual — mostrando os trechos certos, com autoria correta, sem
   nenhuma lista.
2. `Ctrl+t V`, no mesmo projeto com mais de um checkpoint, abre a
   lista com o checkpoint da sessão pré-selecionado; escolher outro
   (mais antigo, ou nomeado pelo usuário) abre a revisão a partir dele.
3. Editar manualmente, por cima da IA, uma faixa dentro de um dos
   trechos (sem passar pelo `aceitar`) faz aquele trecho aparecer como
   "misto" na reabertura da revisão.
4. Aceitar um trecho, fechar o nas, reabrir a mesma revisão: o trecho
   aparece marcado como já revisado.
5. **Duas inserções distintas no mesmo arquivo** (cada uma com o lado
   "antes" vazio, mas textos "depois" diferentes) geram `hunk_hash`
   diferentes; aceitar uma não marca a outra como revisada.
6. Rejeitar um trecho cujo texto ainda é o que a IA deixou aplica `u`
   e desfaz só aquele trecho, journalado como `Author::User` (comando
   `review-reject`) — e essa rejeição, por sua vez, é desfazível com
   `u` no arquivo real, como qualquer edição do usuário; rejeitar um
   trecho já alterado por outra edição recusa com uma mensagem, sem
   tocar no arquivo.
7. `h`/`H` navegam entre trechos e arquivos sem jamais mudar o
   significado de `j k l ; J : K L` em `Review` comparado a `Visual`;
   remapear um desses movimentos em `keymap.toml` continua valendo
   dentro da revisão, sem precisar editar `[review]`.
8. `g h`, fora de qualquer revisão, não tem efeito (sem trecho atual);
   dentro da revisão, abre o arquivo real na linha do trecho.
9. `reviews_log.jsonl` registra início, fim e as contagens de uma
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
- Prepara terreno pra issue #9 sem implementá-la por completo:
  `Review` constrói, pela primeira vez no nas, uma camada de teclas de
  verdade (`[review]` sobre `[visual]` resolvido, bloqueio por nome
  de ação) — não um modo isolado que a issue #9 precisaria
  reconstruir do zero, mas o primeiro caso real que ela generaliza
  (de uma camada fixa pra N camadas declaradas como dados, com
  contexto e resolução de conflito), sem precisar mudar o que
  `Review` *faz*, só como ela está montada.
- `[prefixes.go]` ganha uma entrada (`h`) que só tem efeito dentro da
  revisão — primeira vez que uma entrada de um prefixo global é
  condicional ao estado de um jeito visível pro usuário (fora disso,
  os prefixos de hoje sempre fazem algo quando a tecla existe na
  tabela).
