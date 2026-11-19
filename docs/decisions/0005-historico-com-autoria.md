# 0005 — histórico com autoria e checkpoints nomeados

## Status

Aceita. Implementação em quatro etapas, cada uma na sua própria
branch e PR: 2a (crate `nas-history` com autoria e journal, ligado ao
`nas-editor`/`nas-term`) e 2b (checkpoints, armazenamento por hash e
poda) e 2c (diff entre checkpoints, restauração e as teclas `Ctrl+t`)
feitas. Falta 2d: detecção de mudança externa (`Author::External`).

## Contexto

O README lista quatro pilares. O terceiro diz: "Mudanças da IA são
transações. Tudo que a IA avalia vira um changeset: ver o diff,
desfazer, ou promover com commit no Git." Hoje isso não existe — cada
`Document` só tem o desfazer linear do `helix_core::history::History`,
por buffer, sem nenhuma noção de quem fez uma edição, quando, ou por
qual comando. Não há nada que sobreviva a fechar o nas.

A IA como autora só chega na fase 3 (ainda não tem Lisp embutido, nem
`defcommand`, nem a própria IA). Mas o campo de autoria precisa nascer
agora, porque o histórico de hoje em diante já deveria registrar quem
fez cada mudança — inclusive as que o próprio usuário faz à mão, e as
que acontecem fora do nas — para que, quando a IA chegar, não seja
preciso reescrever o histórico já acumulado nem o formato em disco.

## Decisão

### 1. Autoria

Toda transação que muda o texto registra três coisas: **quem** fez
(`Author`), **quando** (um timestamp de parede, não um `Instant` —
precisa sobreviver ao processo) e **qual comando** gerou a mudança
(uma string, usando os mesmos nomes canônicos que `docs/keys.md` já
define em `[commands]` — ex. `"delete"`; um verbo+objeto vira
`"delete:word-forward"`, verbo e objeto separados por `:`).

```rust
pub enum Author {
    User,
    Ai { agent: String, session: String },
    Extension(String),
    External,
}
```

- **`User`**: a pessoa editando direto.
- **`Ai { agent, session }`**: `agent` é o nome de quem está agindo —
  `"tia"`, `"claude-code"` — e `session` identifica a sessão ou tarefa,
  pra distinguir duas rodadas do mesmo agente (ou a mesma tarefa
  reaberta depois). A IA de verdade só chega na fase 3; até lá,
  `Author::Ai` só aparece em teste, com um autor simulado — ex.
  `Author::Ai { agent: "teste".into(), session: "sessao-1".into() }`
  — provando que o campo funciona de ponta a ponta sem precisar de
  nenhuma IA real.
- **`Extension(String)`**: nome de uma extensão/plugin (um
  formatador, uma code action de LSP), quando existirem.
- **`External`**: uma mudança no arquivo que não veio de nenhum dos
  três acima — alguém rodou `git checkout`, um formatador rodou fora
  do nas, outro programa editou o mesmo arquivo. **Detecção**: esta
  proposta não monitora arquivos continuamente (nada de `inotify`/
  `notify` ainda — fica pra depois, se precisar). Em vez disso, o nas
  compara mtime e tamanho do arquivo no disco contra o que ele mesmo
  leu ou escreveu por último, em quatro momentos: **ao trocar pra um
  buffer** (`Ctrl+b`, ou reabrir); **imediatamente antes de salvar**
  (`Ctrl+s`); **quando o terminal recupera o foco** (eventos
  `FocusGained`/`FocusLost` do crossterm — o caso mais comum de um
  editor externo ou um `git checkout` acontecerem enquanto o usuário
  estava noutra janela); e **de forma leve, a cada poucos segundos**
  (um `stat` por buffer aberto, não uma releitura de conteúdo — barato
  mesmo com vários buffers abertos).

  Ao detectar: **avisa na barra de status e oferece recarregar** — não
  recarrega sozinho, porque o usuário pode ter uma edição em andamento
  que não quer perder. **Nunca salva por cima de uma mudança externa
  sem perguntar**: se `Ctrl+s` detecta que o arquivo mudou no disco
  desde a última leitura/escrita do nas, pergunta antes de sobrescrever
  — mesmo padrão de confirmação que `Ctrl+q`/`Ctrl+k` já usam. Só
  depois de resolvido (recarregar, ou confirmar a sobrescrita) é que a
  próxima edição local journala normalmente; a mudança externa em si
  já entrou no journal como `Author::External` no momento em que foi
  detectada, calculando a diferença contra o que o nas tinha.

Só **transações que de fato mudam o texto** entram no histórico — o
mesmo conjunto de operações que já passa por `Document::apply()` hoje
(inserir, apagar, colar, restaurar um checkpoint). Mover o cursor,
abrir um prompt, trocar de buffer — nada disso gera uma transação, e
isso já é verdade no código atual, sem precisar de nenhuma mudança:
`apply()` só é chamado por operações que de fato tocam o `Rope`.

`undo` (`u`) **também** é journalado: ele muda o texto de volta, então
precisa aparecer no histórico com o autor de quem apertou `u` — senão
"quem fez a mudança atual" ficaria errado depois que alguém desfizer
uma edição de outro autor.

### 2. Granularidade: agrupar digitação contínua, gravar em lotes

A granularidade do **desfazer** (`u`) não muda — continua por tecla,
exatamente como hoje (o helix já comita uma revisão por chamada de
`apply()`). O que muda é só o **journal**: ele não precisa espelhar
essa granularidade.

**Regra de agrupamento.** Duas transações consecutivas se juntam numa
única entrada do journal se, entre as duas:

- o autor, o comando e o arquivo são os mesmos;
- a posição em que a segunda começa é exatamente onde a primeira
  terminou (edição contígua — digitar ou apagar seguindo o cursor,
  nunca um salto);
- menos de 2 segundos se passaram entre as duas.

Qualquer uma dessas condições falhando fecha a entrada atual (ela vai
pra fila de escrita) e abre uma nova. Trocar de arquivo, de comando
(inserir virar apagar, por exemplo) ou de autor sempre fecha. Criar um
checkpoint, fechar o buffer, ou fechar o nas também força o
fechamento de qualquer entrada ainda aberta.

Isso resolve o caso que motivou a pergunta: digitar um parágrafo
inteiro, numa sessão contínua, vira **uma entrada**, não uma por
tecla — e ainda é exatamente isso que `Ctrl+t a` (autoria, ver §8) ia
mostrar de qualquer forma, então agrupar na gravação não perde
informação de autoria nenhuma, só economiza linhas.

**Gravação em lotes, nunca uma escrita por tecla.** Entradas fechadas
vão pra uma fila em memória; a fila é escrita (append) no
`journal.jsonl` a cada 1 segundo ou a cada 50 entradas fechadas, o que
vier primeiro. Fechar o nas, fechar um buffer ou criar um checkpoint
força o flush imediato da fila **e** de qualquer entrada ainda aberta
— um checkpoint, em particular, precisa disso: o snapshot que ele cria
tem que refletir o journal até aquele instante.

O pior caso de perda, se o processo for encerrado abruptamente (não
um fechamento normal), é até ~1 segundo de digitação ou 50 entradas, o
que for menor — e só a entrada de **autoria** dessas últimas edições.
O texto em si não se perde mais do que já se perderia em qualquer
editor sem salvar, por um motivo completamente diferente e já
existente hoje.

### 3. Conteúdo do journal: guarda o texto, não só metadados

Cada entrada guarda o suficiente pra **reconstruir o texto**, não só
quem/quando/o quê em abstrato:

```json
{"ts_start": 1732000000, "ts_end": 1732000001, "author": {"kind": "user"}, "command": "delete", "file": "crates/foo/src/lib.rs", "from": 120, "to": 123, "removed": "abc", "inserted": ""}
```

`removed` é o texto que estava ali antes (vazio numa inserção pura);
`inserted` é o texto novo (vazio numa remoção pura); os dois juntos,
numa substituição (colar sobre uma seleção, por exemplo). Isso é o
suficiente pra:

- mostrar **quem alterou cada linha** — a última entrada cujo
  intervalo cobre aquela linha dá a autoria; é um `git blame`, só que
  sobre o journal do nas em vez de commits;
- **reconstruir qualquer estado intermediário**, aplicando ou
  invertendo entradas em sequência, sem precisar de um checkpoint
  exatamente naquele ponto.

### 4. Checkpoints nomeados: o projeto, não o arquivo

**Concordo com a recomendação: checkpoint é do projeto, não de um
arquivo.** Um checkpoint nomeado captura, num só momento, o conteúdo
de todo arquivo que mudou desde o checkpoint anterior (ou desde que o
projeto foi aberto, se for o primeiro) — não só o buffer ativo.

A justificativa é a mesma que a pergunta já deu: uma tarefa da IA
(ou do usuário) frequentemente toca mais de um arquivo, e o ponto de
"antes/depois" que faz sentido revisar ou desfazer é o da tarefa
inteira, não de um arquivo isolado. Um checkpoint por arquivo
obrigaria a lembrar manualmente de marcar todos os arquivos tocados,
e dois checkpoints de arquivos diferentes na mesma tarefa não teriam
como ser vistos juntos depois. Isso também casa com o pilar 3 do
README ("tudo que a IA avalia vira um changeset") — um changeset já é,
por definição, um conjunto de arquivos.

Um checkpoint captura o conteúdo **atual de todo arquivo rastreado**
— não precisa de `Ctrl+s` antes, nem de o arquivo ter um buffer aberto
(vem do buffer, se houver um; senão, do disco). Isso é deliberado:
checkpoint é um mecanismo do nas, anterior e independente de salvar no
disco do projeto ou de um commit no Git (que continua sendo decisão
do usuário, promovida manualmente — pilar 3). **O histórico nunca
toca o `.git` do projeto.**

**Correção (a versão original desta ADR dizia "só os arquivos que
mudaram desde o checkpoint anterior" — errado):** um checkpoint
registra o estado de **todo arquivo rastreado até ali**, não só os
que mudaram desde o anterior. Rastreado = já teve ao menos uma entrada
no journal, nesta sessão ou numa anterior. A razão é restaurar: se um
checkpoint só lista arquivos que mudaram desde o anterior, e um
arquivo `y` é editado por primeira vez *depois* de um checkpoint `A`
(mas antes de restaurá-lo), `A` nunca chega a mencionar `y` — e
restaurar `A` precisa, mesmo assim, voltar `y` ao estado que tinha
quando `A` foi criado. A correção tem duas partes:

- **Original capturado na primeira edição.** Na primeira vez que um
  arquivo é rastreado (sua primeira entrada no journal), o conteúdo
  que já estava no disco é capturado e guardado em `objects/` (mesmo
  critério de tamanho/UTF8 que os checkpoints já usam) — nunca
  sobrescrito depois. É "como o arquivo estava" no instante em que o
  nas passou a prestar atenção nele.
- **Todo checkpoint registra todo arquivo rastreado**, reaproveitando
  o hash de `objects/` quando o conteúdo não mudou desde o checkpoint
  anterior — o armazenamento por conteúdo já garante que isso não
  duplica nada em disco, só mais uma linha no manifesto.

Com isso, restaurar um checkpoint `A` sabe exatamente o que fazer com
**todo** arquivo rastreado, não só os que `A` lista: se está no
manifesto de `A`, usa o conteúdo de lá; se não está (foi rastreado
*depois* de `A`), usa o original capturado na primeira edição —
exatamente o estado do arquivo no instante de `A`, já que nada o tocou
antes daquela primeira edição. Ver §7.

### 5. Persistência

**Sobrevive a fechar o nas.** Fica em `~/.local/state/nas/`
(`$XDG_STATE_HOME`, com o mesmo fallback manual que
`NAS_KEYBOARD_LAYOUT` já usa pra variável de ambiente — sem nova
dependência de diretórios), separado por projeto (ver §6 pra como o
projeto é identificado):

```
~/.local/state/nas/history/<hash-do-projeto>/
  project.toml              # { path = "/caminho/absoluto/do/projeto" }
  journal.jsonl             # uma linha JSON por entrada, append-only
  objects/
    <hash-do-conteúdo>      # conteúdo de um arquivo num certo momento
  checkpoints/
    <id>/
      manifest.toml         # nome, criado_em, autor, automatico?, arquivos
```

- **Permissão**: `~/.local/state/nas/history/` e cada
  `<hash-do-projeto>/` são criados com `0700` (só o dono lê/escreve/
  entra) — este histórico guarda conteúdo de texto do usuário (e,
  depois, da IA), merece a mesma cautela que `~/.ssh` ou um
  `.netrc`.
- **`<hash-do-projeto>`**: hash curto (ex. os 16 primeiros hex do
  SHA-256) do caminho raiz do projeto (§6). `project.toml` guarda o
  caminho de verdade, pra inspeção manual sem decifrar o hash.
- **`journal.jsonl`**: formato próprio (§3) — **nunca** serializando
  `helix_core::Transaction` direto; a API do helix não é estável
  (decisão 1), e este journal precisa continuar legível entre versões
  do nas que atualizem a tag do helix.
- **Leitura tolerante a linha incompleta**: se o processo for
  encerrado abruptamente no meio de um append (§2), a **última** linha
  do `journal.jsonl` pode ficar truncada — um objeto JSON incompleto.
  Ao ler, o nas tenta parsear cada linha; se **só a última** falhar,
  ela é ignorada, como se nunca tivesse sido escrita — o resto do
  arquivo continua válido. Uma falha em qualquer linha que não seja a
  última é um erro de verdade (não deveria acontecer, já que todo
  append anterior já tinha sido concluído por inteiro antes do
  próximo começar) e não é tolerada do mesmo jeito.
- **`objects/`**: um **armazenamento endereçado por conteúdo**. Cada
  arquivo snapshot (de um checkpoint) é guardado uma vez só, nomeado
  pelo hash do seu próprio conteúdo (SHA-256); se o mesmo conteúdo
  aparece em dois checkpoints — ou volta a aparecer depois de ter
  mudado e voltado — não é duplicado, o manifesto dos dois checkpoints
  só referencia o mesmo objeto. Isso responde à pergunta: **nunca
  duplicar versões iguais entre checkpoints.**
- **Limite de tamanho**: arquivos maiores que **2 MiB**, ou que não
  são UTF-8 válido (o mesmo critério que `Document::open` já usa pra
  recusar abrir um arquivo como texto), não são copiados pra
  `objects/` — o manifesto do checkpoint ainda registra que aquele
  caminho foi tocado (nome, tamanho, hash do conteúdo real no disco,
  pra detectar se mudou depois), só não guarda o conteúdo. Diff e
  restauração pra esses arquivos ficam fora do que esta proposta
  cobre — binários grandes não são o caso de uso do histórico de
  texto. **Quando um checkpoint deixa algum arquivo de fora por isso,
  o nas avisa quais ficaram de fora** (nome e o motivo — grande demais
  ou não-UTF8) — nunca falha silenciosamente nem finge que o
  checkpoint capturou tudo.
- **`checkpoints/<id>/manifest.toml`**: nome, quando foi criado,
  quem criou, se é **automático** (ver poda, abaixo) e a lista de
  arquivos com o hash do conteúdo de cada um em `objects/`. `<id>` é
  `<timestamp-unix>-<nome-normalizado>`, ordenável só listando o
  diretório.
- **Poda, por dois limites, não só por quantidade**:
  - Checkpoints **nomeados manualmente pelo usuário** (toda criação
    via `Ctrl+t n`) **nunca são podados automaticamente**.
  - Checkpoints **automáticos** (hoje, só o "antes de restaurar" que
    §7 cria) são os primeiros candidatos à poda, por idade, antes de
    qualquer nomeado — reter por padrão os **20 automáticos mais
    recentes por projeto**.
  - Além disso, um **limite de tamanho total em disco por projeto**
    (proponho **500 MiB**, incluindo `objects/`): ao exceder, remove
    os automáticos mais antigos primeiro; se exceder mesmo depois de
    remover todos os automáticos, para e avisa o usuário — nunca
    remove um checkpoint nomeado só por pressão de espaço (apagar um
    nomeado específico é uma ação futura, de escolha explícita, não
    coberta em detalhe aqui).
  - O **journal** é podado mantendo só entradas mais novas que o
    checkpoint mais antigo ainda vivo (nomeado ou automático) —
    entradas mais velhas não servem pra nenhum diff/restauração mais
    possível.
  - Toda poda de checkpoint também precisa **coletar objetos órfãos**
    em `objects/` — um conteúdo que não é mais referenciado por
    nenhum manifesto sobrevivente é removido também, senão o
    deduplicar do parágrafo anterior não economiza nada no fim.
  - Os números (20, 500 MiB, 2 MiB, 2 segundos, 1 segundo/50 entradas)
    são pontos de partida razoáveis, não uma medição — ajustar depois
    é mudar uma constante, não um redesenho.
- **Privacidade e apagar o histórico**: um comando (`Ctrl+t z`, §8)
  remove o `<hash-do-projeto>/` inteiro — journal e todos os
  checkpoints, nomeados ou não. **Até esse comando ser usado, texto
  apagado do buffer continua existindo no histórico** — apagar uma
  linha no editor não apaga o rastro dela no journal nem em nenhum
  checkpoint que a tenha capturado. Isso é o propósito do histórico,
  mas é a mesma pegadinha que o Git já tem (reverter um commit não
  apaga o commit) — só que aqui não existe um `git log` visível pra
  lembrar o usuário disso, então vale deixar bem documentado (e, no
  futuro, talvez visível na própria UI de histórico).

### 6. Identificação do projeto

O projeto é identificado pela **raiz do repositório Git**, se o
diretório onde o nas foi iniciado estiver dentro de um — procurada
subindo os diretórios a partir de onde o nas foi iniciado até achar um
`.git`. Sem um repositório Git, é o próprio diretório onde o nas foi
iniciado. Isso só **lê** a presença do `.git` pra decidir a raiz —
nunca escreve nem roda nenhum comando `git`; continua valendo "o
histórico nunca toca o `.git` do projeto".

A raiz escolhida (Git ou não) é o caminho que entra no hash
(`<hash-do-projeto>`) e em `project.toml`.

**Se a pasta do projeto for movida ou renomeada**, o caminho absoluto
muda, o hash muda, e o histórico antigo fica **órfão** — associado a
um caminho que não existe mais, mas não apagado (nada remove
automaticamente uma pasta de histórico por isso). Esta proposta aceita
essa limitação de propósito: resolver de verdade pediria uma
identidade mais estável que um caminho — algo do próprio Git (que
exigiria rodar `git`, não só ler o `.git`) ou um marcador escrito
dentro do projeto (que tornaria o nas visível demais fora de
`~/.local/state`). Por ora, a recuperação é manual — renomear a pasta
correspondente dentro de `~/.local/state/nas/history/`, ou aceitar
começar um histórico novo. `project.toml` guarda o caminho antigo, o
que deixa aberto, no futuro, detectar "esta pasta de histórico aponta
pra um caminho que não existe mais" e oferecer adoção pela nova
localização — não é parte desta proposta.

### 7. Diff entre checkpoints e restauração

**Diff** entre dois checkpoints: para cada arquivo presente em
qualquer um dos dois, compara o conteúdo guardado em `objects/` (via
o hash de cada um no manifesto) — ou a ausência, se o arquivo só
existe num dos dois. É um diff de texto comum, sem nenhuma dependência
do helix — proponho a crate [`similar`](https://docs.rs/similar) (diff
de linhas, madura, usada amplamente) em vez de reimplementar Myers
diff. É a única dependência nova que esta proposta pede.

**Restaurar** um checkpoint volta **o projeto inteiro** ao estado
daquele momento — não só os arquivos que o manifesto dele lista
(correção, ver §4): para cada arquivo **rastreado**,

- se está no manifesto do checkpoint sendo restaurado, usa o conteúdo
  de lá;
- se não está (foi rastreado só *depois* dele), mas **existia** no
  disco antes de ser rastreado, volta ao conteúdo original capturado
  na primeira edição (§4) — exatamente o estado no instante do
  checkpoint;
- se não está, e **não existia** (foi criado depois do checkpoint),
  vai pra **lixeira do sistema** (`~/.local/share/Trash`) — nunca
  apagado de vez. A confirmação antes de restaurar lista esses
  arquivos.

Antes de tocar em qualquer arquivo, a restauração **cria
automaticamente um checkpoint novo** (marcado como automático, §5)
com o estado atual de **todo arquivo rastreado** (nome gerado, ex.
`"antes de restaurar <nome>"`, autor = quem disparou a restauração).
Isso responde a "o que acontece com mudanças feitas depois" (nada se
perde — ficam naquele checkpoint automático, inclusive o que seria
revertido ao original ou jogado na lixeira) e a "restaurar precisa ser
desfazível" ao mesmo tempo, com o mesmo mecanismo: desfazer uma
restauração é só restaurar o checkpoint automático que ela mesma
criou. Nenhum código separado de "desfazer restauração" — e nada que
foi pra lixeira precisa ser recuperado de lá manualmente, já que o
automático guarda o conteúdo de qualquer forma.

Nenhum erro de gravação é ignorado: se escrever um arquivo falhar
(permissão, disco cheio), a restauração continua com os outros e
reporta, ao final, quais falharam e por quê — nunca finge sucesso.

**Buffers abertos**: os buffers correspondentes a um arquivo do
checkpoint sendo restaurado são **recarregados** com o conteúdo
restaurado. Se algum desses buffers tiver alterações não salvas no
momento da restauração, o nas **pergunta antes de restaurar**,
listando quais buffers seriam substituídos — mesmo padrão de
confirmação que `Ctrl+q`/`Ctrl+k` já usam. Isso não é sobre perder
dados de verdade (o checkpoint automático já garante que nada se
perde, nem o que só estava na memória) — é sobre não trocar o
conteúdo debaixo do usuário, sem aviso, enquanto ele ainda está
olhando ou editando aquele buffer. A mesma confirmação também lista os
arquivos que vão pro original ou pra lixeira (acima) — as duas coisas
pedem confirmação juntas, numa pergunta só.

A escrita de cada arquivo restaurado passa pelo `Document::apply()`
normal (como uma transação "substituir tudo"), então também entra no
undo por tecla (`u`) do buffer e no journal, com comando
`"restore-checkpoint"`.

### 8. Teclas (proposta, nada implementado)

Histórico e checkpoint são o editor **como programa** sobre o projeto
inteiro — classe sistema (Ctrl), não edição de texto. Um atalho único
de sistema por ação ficaria sem letras livres rápido, então proponho
um **prefixo de sistema novo**: `Ctrl+t` ("tempo"), esperando mais uma
tecla **sem Ctrl**, do mesmo jeito que `g` e `Espaço` já são prefixos
hoje — só que entrando pela classe sistema em vez do estado Visual.
Isso é uma extensão pequena do mecanismo de `[system]` em `nas-keys`
(hoje resolve só uma tecla, nunca fica pendente); esta proposta não
implementa essa extensão, só assume que ela existe.

| Teclas | Ação | Por que essa letra |
|---|---|---|
| `Ctrl+t n` | novo checkpoint nomeado (pede o nome num prompt) | "novo" — livre: não é verbo, não está em `[system]` |
| `Ctrl+t l` | listar checkpoints do projeto | "listar" — `l` só significa "linha acima"/"parágrafo anterior" dentro da classe texto (estado Visual); `Ctrl+t` é outra classe (sistema), reaproveitar a letra não reaproveita o significado, o mesmo raciocínio que já vale pra `Ctrl+s` (salvar) não ter nada a ver com `s` solto (marcar seleção) |
| `Ctrl+t e` | diff **e**ntre dois checkpoints (ou um checkpoint e agora) | evitei `d` **de propósito** — `d` já é o verbo "apagar", carregado o bastante pra que `Ctrl+t d` fosse lido por reflexo como algo destrutivo, quando diff não apaga nada |
| `Ctrl+t r` | restaurar um checkpoint escolhido | "restaurar" — livre, sem colisão |
| `Ctrl+t a` | **a**utoria (blame) da linha/região atual | evitei `b` **de propósito** — `b` já é `Ctrl+b` (trocar de buffer), e as duas teclas compartilham o mesmo modificador (Ctrl), então a chance de confundir uma com a outra é maior do que entre classes diferentes, como no caso de `l` acima |
| `Ctrl+t z` | apagar o histórico deste projeto (§5) | sem mnemônico de propósito — é uma ação rara e destrutiva; uma letra sem associação prévia reduz a chance de ser apertada por hábito |

`Ctrl+t z` pede uma confirmação **mais forte** que a tecla repetida
(`d a`, `x a`): um prompt pedindo pra **digitar "apagar"** antes de
executar. A tecla repetida já é suficiente pra ações que o próprio
histórico protege (apagar o arquivo inteiro ainda fica recuperável
via checkpoint); apagar o histórico em si remove essa rede de
segurança — não há nenhum checkpoint "antes de apagar o histórico"
pra desfazer isso depois. Uma barreira maior pra uma ação sem rede.

`Ctrl+c` fica de fora de propósito — é universalmente SIGINT no
terminal; mesmo em modo raw, onde o nas poderia interceptar, seria uma
surpresa desagradável sobrescrever essa expectativa. `Ctrl+h` já é
proibido por `docs/keys.md` §5.7 (colide com Backspace em alguns
terminais) — mas isso é sobre `Ctrl+h` sozinho; `Ctrl+t` seguido de uma
letra sem Ctrl não tem esse problema, porque a segunda tecla é só uma
letra comum, sem ambiguidade de controle.

Nota: `Espaço t` já está reservado (`docs/keys.md` §6) para falar com
a IA — é uma tecla diferente (líder, não sistema), mas o mesmo "t" em
dois contextos é uma coincidência de mnemônica (**t**empo vs. **t**alk)
que vale apontar, não necessariamente evitar.

### 9. Arquitetura: uma crate nova, `nas-history`

Proponho uma crate nova, `crates/nas-history`, que **não depende de
`nas-editor` nem de `nas-keys`** — só de tipos simples (`String`,
`PathBuf`, timestamps, intervalos de caracteres). Ela é dona de
`Author`, da entrada do journal (agrupamento e fila de escrita em
lotes, §2), do formato em disco (§5), do armazenamento endereçado por
conteúdo e da poda (§5), e do diff (via `similar`, §7).

`nas-term` chama `nas-history` do mesmo jeito que já chama
`nas-editor` e `nas-keys` — é a única crate que conhece as três. Cada
operação de `Document` que já passa por `apply()` ganha, do lado de
`nas-term`, uma chamada pra registrar a transação equivalente no
journal (autor, comando, arquivo, intervalo, texto removido/inserido).
Isso evita que `nas-editor` precise saber o que é um "comando"
(conceito de `nas-keys`) ou o que é "autoria" (conceito novo) — ele
continua só sabendo de texto.

Essa é a única decisão grande e não registrada desta proposta (nova
crate) — por isso está aqui, não direto em código, seguindo a regra do
`CLAUDE.md` de parar antes de decisões assim.

## Critério de pronto

1. Criar um checkpoint nomeado; fazer mais edições como `Author::User`
   e como um `Author::Ai { agent: "teste", session: "s1" }` simulado
   (sem IA de verdade — só pra provar que o campo funciona); fechar o
   nas; abrir de novo no mesmo projeto; o checkpoint criado antes de
   fechar ainda aparece em `Ctrl+t l`.
2. Restaurar esse checkpoint, horas (ou um fechar/abrir) depois de
   criado: os arquivos voltam ao conteúdo de então; um checkpoint
   automático "antes de restaurar" aparece na lista; restaurar esse
   automático desfaz a restauração.
3. `Ctrl+t e` entre dois checkpoints mostra a diferença de texto real
   entre eles.
4. `Ctrl+t a` numa linha qualquer, depois do passo 1, mostra
   corretamente quem fez aquela mudança (usuário ou o autor simulado),
   com qual comando e quando — mesmo depois do fechar/abrir do passo 1.
5. Um `journal.jsonl` com a última linha truncada (simulando um
   encerramento abrupto no meio de um append) ainda carrega todas as
   entradas anteriores corretamente ao abrir o projeto de novo — só a
   última linha é ignorada.

## Consequências

- `Document` ganha pelo menos um método novo de conveniência
  (substituir o conteúdo inteiro de uma vez, pra restauração) — ainda
  dentro do escopo atual dele (texto), sem nenhuma mudança de
  autoria/comando vazando pra `nas-editor`.
- `nas-history` precisa manter, em memória, uma entrada "em aberto"
  por (autor, arquivo) pra poder agrupar digitação contínua (§2) — é
  estado novo, mas fica contido dentro da crate nova, sem vazar pra
  `nas-term`.
- Deduplicar conteúdo por hash (§5) só economiza espaço de verdade se
  a poda de checkpoints também coletar `objects/` órfãos — as duas
  partes (dedup e coleta) precisam ser implementadas juntas, nunca só
  uma.
- Detectar `Author::External` (§1) significa que trocar de buffer e
  salvar ganham uma checagem de mtime/tamanho a mais — barata (um
  `stat`), mas é um comportamento novo que não existe hoje em
  `handle_system_key`/`save`. Os outros dois momentos de checagem
  (foco do terminal, verificação periódica) exigem mudanças no loop
  de eventos de `main.rs`: habilitar eventos de foco do crossterm
  (`EnableFocusChange`, ao lado do bracketed paste que já é
  habilitado) e trocar a leitura hoje bloqueante (`event::read()`) por
  uma com tempo limite (`event::poll`) ou um tick do `tokio`, pra ter
  uma chance de checar periodicamente mesmo sem nenhuma tecla chegar.
- Apagar o histórico (`Ctrl+t z`) com confirmação digitada é um tipo
  de prompt novo pra `nas-term` — os prompts de hoje (`PromptKind`)
  só pedem um valor (caminho, consulta, número de linha); este
  precisa comparar o que foi digitado contra um valor fixo
  (`"apagar"`) antes de agir, em vez de usar o valor digitado
  diretamente.
- `nas-keys` precisa de uma extensão pequena e genérica (prefixo
  dentro da classe sistema, §8) antes desta proposta poder ser
  implementada de verdade — não é parte desta ADR, mas é um
  pré-requisito dela.
- Quando a fase 3 (IA) chegar, nenhuma mudança de formato é esperada:
  `Author::Ai { agent, session }` já existe e já está testado; só
  passa a ser preenchido por um agente de verdade em vez de um autor
  simulado.
