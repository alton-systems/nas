# 0005 — histórico com autoria e checkpoints nomeados

## Status

Proposta — aguardando aprovação. Nada deste documento está implementado.

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
fez cada mudança — inclusive as que o próprio usuário faz à mão —
para que, quando a IA chegar, não seja preciso reescrever o histórico
já acumulado nem o formato em disco.

## Decisão

### 1. Autoria

Toda transação que muda o texto registra três coisas: **quem** fez
(`Author`), **quando** (um timestamp de parede, não um `Instant`
— precisa sobreviver ao processo) e **qual comando** gerou a mudança
(uma string, usando os mesmos nomes canônicos que `docs/keys.md` já
define em `[commands]` — ex. `"delete"`, `"move-word-forward"` não
entra porque não muda texto, mas `"paste"` entra; um verbo+objeto vira
`"delete:word-forward"`, verbo e objeto separados por `:`).

```rust
pub enum Author {
    User,
    Ai(String),        // nome do agente — "claude", por exemplo
    Extension(String), // nome da extensão/plugin
}
```

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

**Granularidade: uma entrada por transação, sem agrupar.** Hoje,
digitar "abc" já gera três revisões separadas no `History` do helix
(confirmado pelo teste `insere_apaga_e_desfaz_com_transaction_e_history`,
que desfaz as 9 inserções de "ola mundo" uma por uma). Journalar nessa
mesma granularidade é a opção simples — não precisa decidir "quando
agrupar" nem manter um buffer pendente. O custo é verbosidade: uma
sessão de digitação humana gera muitas entradas de um caractere só.
Isso é aceitável porque:

- é exatamente a granularidade que já existe pro desfazer;
- uma edição da IA tende a ser uma `Transaction` só (várias
  inserções/remoções de uma vez, não tecla por tecla) — então entradas
  de IA já nascem mais grossas, sem precisar de nenhum agrupamento;
- agrupar pra exibição (ex. uma visão de autoria mostrando "User
  inseriu 'hello world' às 14:32" em vez de 11 entradas) é uma
  decisão de **como mostrar**, não de **como guardar** — pode vir
  depois, sem mudar o formato em disco.

### 2. Checkpoints nomeados: o projeto, não o arquivo

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

Um checkpoint só inclui arquivos que **mudaram** desde o checkpoint
anterior (têm pelo menos uma entrada no journal nesse intervalo) —
não todo arquivo aberto. Arquivos sem mudança não precisam de uma
cópia nova: o diff contra o checkpoint anterior seria vazio de
qualquer forma.

Um checkpoint captura o conteúdo **atual dos buffers em memória**,
salvos ou não — não precisa de `Ctrl+s` antes. Isso é deliberado:
checkpoint é um mecanismo do nas, anterior e independente de salvar no
disco do projeto ou de um commit no Git (que continua sendo decisão
do usuário, promovida manualmente — pilar 3). O histórico nunca toca
o `.git` do projeto.

### 3. Persistência

**Sobrevive a fechar o nas.** Fica em `~/.local/state/nas/`
(`$XDG_STATE_HOME`, com o mesmo fallback manual que `NAS_KEYBOARD_LAYOUT`
já usa pra variável de ambiente — sem nova dependência de diretórios),
separado por projeto:

```
~/.local/state/nas/history/<hash-do-projeto>/
  project.toml              # { path = "/caminho/absoluto/do/projeto" }
  journal.jsonl             # uma linha JSON por transação, append-only
  checkpoints/
    <id>/
      manifest.toml         # nome, criado_em, autor, lista de arquivos
      files/
        <caminho relativo ao projeto>   # cópia integral do arquivo
        ...
```

- **`<hash-do-projeto>`**: hash curto (ex. os 16 primeiros hex do
  SHA-256) do caminho absoluto canonicalizado do diretório onde o nas
  foi iniciado — o mesmo diretório já usado pra resolver caminhos
  relativos no prompt de abrir arquivo. `project.toml` guarda o
  caminho de verdade, pra inspeção manual sem precisar decifrar o
  hash.
- **`journal.jsonl`**: uma linha por transação, formato próprio —
  **nunca** serializando `helix_core::Transaction` direto. A API do
  helix não é estável (decisão 1) e esse journal precisa continuar
  legível entre versões do nas que atualizem a tag do helix. Cada
  linha é só o que a transação significa, em caracteres, não como o
  helix representa internamente:
  ```json
  {"ts": 1732000000, "author": {"kind": "user"}, "command": "delete", "file": "crates/foo/src/lib.rs", "from": 120, "to": 123, "inserted": ""}
  ```
  Escrita **imediata**: cada transação é apendada e sincronizada no
  momento em que acontece, não em lote — um journal de texto é barato
  de escrever, e a prioridade aqui é não perder uma linha se o
  processo for encerrado abruptamente.
- **`checkpoints/<id>/files/`**: cópias **integrais** dos arquivos
  tocados, por valor, espelhando o caminho relativo do projeto — não
  um diff nem uma referência a entradas do journal. Mais simples de
  restaurar (é só copiar de volta) e de inspecionar com qualquer
  ferramenta externa (`diff`, `cat`), ao custo de espaço. `<id>` é
  `<timestamp-unix>-<nome-normalizado>`, ordenável só listando o
  diretório.
- **Sem limite de crescimento**: reter por padrão os **50 checkpoints
  mais recentes por projeto** e, dentro disso, **podar o journal**
  mantendo só as entradas mais novas que o checkpoint mais antigo
  ainda vivo (entradas mais velhas que isso não servem pra nenhum
  diff/restauração possível). Checkpoints são sempre nomeados
  explicitamente pelo usuário — nenhuma poda poderia distinguir
  "importante" de "descartável", então o critério é só idade/posição,
  nunca o nome. Esses números (50; "mais novo que o checkpoint mais
  antigo") são um ponto de partida razoável, não uma medição — ajustar
  depois é só mudar uma constante, não um redesenho.

### 4. Diff entre checkpoints e restauração

**Diff** entre dois checkpoints: para cada arquivo presente em
qualquer um dos dois, compara o conteúdo salvo nas duas pastas
`files/` (ou a ausência, se o arquivo só existe num dos dois). Isso é
um diff de texto comum, sem nenhuma dependência do helix — proponho a
crate [`similar`](https://docs.rs/similar) (diff de linhas, madura,
usada amplamente) em vez de reimplementar Myers diff. É a única
dependência nova que esta proposta pede.

**Restaurar** um checkpoint sobrescreve, nos buffers abertos (abrindo
os que não estiverem abertos), o conteúdo de cada arquivo **que esse
checkpoint contém** — arquivos fora dele não são tocados. Antes de
sobrescrever qualquer coisa, a restauração **cria automaticamente um
checkpoint novo** com o estado atual desses mesmos arquivos (nome
gerado, ex. `"antes de restaurar <nome>"`, autor = quem disparou a
restauração). Isso responde a "o que acontece com mudanças feitas
depois" (nada se perde — ficam naquele checkpoint automático) e a
"restaurar precisa ser desfazível" ao mesmo tempo, com o mesmo
mecanismo: desfazer uma restauração é só restaurar o checkpoint
automático que ela mesma criou. Nenhum código separado de "desfazer
restauração".

A escrita de cada arquivo restaurado passa pelo `Document::apply()`
normal (como uma transação "substituir tudo"), então também entra no
undo por tecla (`u`) do buffer e no journal, com comando
`"restore-checkpoint"`.

### 5. Teclas (proposta, nada implementado)

Histórico e checkpoint são o editor **como programa** sobre o projeto
inteiro — classe sistema (Ctrl), não edição de texto. Um atalho único
de sistema por ação ficaria sem letras livres rápido, então proponho
um **prefixo de sistema novo**: `Ctrl+t` ("tempo"), esperando mais uma
tecla **sem Ctrl**, do mesmo jeito que `g` e `Espaço` já são prefixos
hoje — só que entrando pela classe sistema em vez do estado Visual.
Isso é uma extensão pequena do mecanismo de `[system]` em
`nas-keys` (hoje resolve só uma tecla, nunca fica pendente); esta
proposta não implementa essa extensão, só assume que ela existe.

| Teclas | Ação |
|---|---|
| `Ctrl+t n` | novo checkpoint nomeado (pede o nome num prompt) |
| `Ctrl+t l` | listar checkpoints do projeto |
| `Ctrl+t d` | diff entre dois checkpoints (ou um checkpoint e agora) |
| `Ctrl+t r` | restaurar um checkpoint escolhido |
| `Ctrl+t b` | autoria (blame) da linha/região atual |

`Ctrl+c` fica de fora de propósito — é universalmente SIGINT no
terminal; mesmo em modo raw, onde o nas poderia interceptar, seria uma
surpresa desagradável sobrescrever essa expectativa. `Ctrl+h` já é
proibido por `docs/keys.md` §5.7 (colide com Backspace em alguns
terminais).

Nota: `Espaço t` já está reservado (`docs/keys.md` §6) para falar com
a IA — é uma tecla diferente (líder, não sistema), mas o mesmo "t"
em dois contextos é uma coincidência de mnemônica (**t**empo vs.
**t**alk) que vale apontar, não necessariamente evitar.

### 6. Arquitetura: uma crate nova, `nas-history`

Proponho uma crate nova, `crates/nas-history`, que **não depende de
`nas-editor` nem de `nas-keys`** — só de tipos simples (`String`,
`PathBuf`, timestamps, intervalos de caracteres). Ela é dona de
`Author`, da entrada do journal, do formato em disco, da
leitura/escrita de `journal.jsonl` e dos checkpoints, da poda, e do
diff (via `similar`).

`nas-term` chama `nas-history` do mesmo jeito que já chama
`nas-editor` e `nas-keys` — é a única crate que conhece as três.
Cada operação de `Document` que já passa por `apply()` ganha, do lado
de `nas-term`, uma chamada pra registrar a transação equivalente no
journal (autor, comando, arquivo, intervalo, texto inserido). Isso
evita que `nas-editor` precise saber o que é um "comando" (conceito
de `nas-keys`) ou o que é "autoria" (conceito novo) — ele continua só
sabendo de texto.

Essa é a única decisão grande e não registrada desta proposta (nova
crate) — por isso está aqui, não direto em código, seguindo a regra do
`CLAUDE.md` de parar antes de decisões assim.

## Critério de pronto

1. Criar um checkpoint nomeado; fazer mais edições como `Author::User`
   e como um `Author::Ai("teste")` simulado (sem IA de verdade — só
   pra provar que o campo funciona); fechar o nas; abrir de novo no
   mesmo projeto; o checkpoint criado antes de fechar ainda aparece em
   `Ctrl+t l`.
2. Restaurar esse checkpoint, horas (ou um fechar/abrir) depois de
   criado: os arquivos voltam ao conteúdo de então; um checkpoint
   automático "antes de restaurar" aparece na lista; restaurar esse
   automático desfaz a restauração.
3. `Ctrl+t d` entre dois checkpoints mostra a diferença de texto real
   entre eles.
4. `Ctrl+t b` numa linha qualquer, depois do passo 1, mostra
   corretamente quem fez aquela mudança (usuário ou o autor simulado),
   com qual comando e quando — mesmo depois do fechar/abrir do passo 1.

## Consequências

- `Document` ganha pelo menos um método novo de conveniência
  (substituir o conteúdo inteiro de uma vez, pra restauração) — ainda
  dentro do escopo atual dele (texto), sem nenhuma mudança de
  autoria/comando vazando pra `nas-editor`.
- O journal cresce rápido (uma linha por tecla digitada). A poda por
  checkpoint (§3) é o que mantém isso limitado; sem nenhum checkpoint
  criado, o journal cresce sem limite — vale documentar isso pro
  usuário quando a feature existir (criar checkpoints de vez em
  quando já limita o histórico).
- `nas-keys` precisa de uma extensão pequena e genérica (prefixo
  dentro da classe sistema) antes desta proposta poder ser
  implementada de verdade — não é parte desta ADR, mas é um
  pré-requisito dela.
- Quando a fase 3 (IA) chegar, nenhuma mudança de formato é esperada:
  `Author::Ai(nome)` já existe e já está testado; só passa a ser
  preenchido por um agente de verdade em vez de um autor simulado.
