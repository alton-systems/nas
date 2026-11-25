# 0006 — a IA trabalha pelo nas

## Status

Proposta — aguardando aprovação. Nada implementado; nenhum PR de
código abre a partir desta ADR até ela ser aceita.

## Contexto

O README lista quatro pilares e um roteiro em quatro marcos: Marco 1
(editor básico + modal, em andamento), Marco 2 (Steel/Scheme embutido),
Marco 3 (`defcommand` com esquema/docstring/efeitos, JSON Schema
automático), Marco 4 (a IA propõe uma sequência de comandos, o usuário
aprova no estilo Sequential). A ADR 0005 já adiantou, de propósito, a
metade do pilar 3 que não depende de IA nenhuma: todo `Document::apply`
já registra autor, comando e intervalo; `Author::Ai { agent, session }`
já existe no tipo e já é testado com um autor simulado; checkpoint,
diff e restauração já funcionam para qualquer autor. Essa fase 3 é
sobre a outra metade: ligar um agente de IA de verdade a tudo isso.

**Esta proposta muda o caminho até lá.** O roteiro original chega à IA
via Lisp embutido primeiro (Marco 2) e só depois `defcommand`/esquema
(Marco 3). Proponho inverter: um **registro de comandos nativo em
Rust**, sem precisar de nenhum interpretador embutido, que já nasce
gerando esquema JSON — e um agente de IA de verdade (Claude Code, hoje)
chamando esses comandos via MCP, testável nesta fase, sem esperar o
Steel. Lisp não é descartado: continua fazendo sentido como camada de
*scripting* do usuário mais adiante (e pode, no futuro, chamar os
mesmos comandos do registro) — só deixa de ser pré-requisito pra IA
ser usuária do nas. `nas-commands` (hoje reservada pro Marco 3 em
`CLAUDE.md`) passa a nascer agora, antes de `nas-lisp`. **Essa é a
única decisão grande e não registrada desta proposta** — inversão de
ordem do roteiro — e é por isso que esta ADR existe antes de qualquer
código, seguindo a regra do `CLAUDE.md` de parar antes de decisões
assim.

## Decisão

### 1. Registro de comandos tipados

Uma crate nova, `nas-commands`, análoga a `nas-history` (ADR 0005 §9):
não depende de `nas-keys` nem sabe de teclas — só dos tipos de
domínio (`nas-editor::Document`, `nas-history::Journal`). Um comando é
uma função Rust com uma assinatura fixa, mais os quatro metadados do
pilar 1 do README:

```rust
pub enum Effect {
    Pure,
    ReadFile,
    WriteBuffer,
    /// Escreve no armazenamento do histórico (`~/.local/state/nas/`)
    /// — nunca no arquivo do projeto em si. Separado de `WriteFile`
    /// porque o risco é outro: `checkpoint_create` só acrescenta um
    /// snapshot ao histórico, sempre reversível (apagar um
    /// checkpoint não é uma operação que exista ainda) — ver §5.
    WriteHistory,
    WriteFile,
    Shell,
    Network,
}

pub struct CommandSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub effect: Effect,
    pub args_schema: schemars::Schema,
    run: Box<dyn Fn(&mut CommandContext, serde_json::Value) -> Result<CommandOutput, CommandError>>,
}

pub struct Registry(Vec<CommandSpec>);
```

Cada comando declara seus argumentos como uma struct comum
(`#[derive(Deserialize, schemars::JsonSchema)] struct EditArgs { path: String, from: usize, to: usize, text: String }`),
e `args_schema` vem de `schemars::schema_for!` — sem escrever nenhum
JSON Schema à mão. Registrar um comando é uma função livre
(`fn register(registry: &mut Registry)`, chamada uma vez por comando
dentro de um `register_all` central) — nada de macro (`defcommand` é
Lisp, Marco 3 original; aqui ainda é só Rust simples, sem abstração
antecipando o que ainda não existe, por `CLAUDE.md`).

**Isto cria, de propósito, dois sistemas de comando que ainda não se
falam** — uma duplicação reconhecida, não um descuido:
`nas-keys`/`[commands]` liga uma tecla a uma ação síncrona, sem
argumentos, só pro estado Visual/Sequential; `nas-commands` é
invocável por nome com argumentos explícitos, hoje só por um agente
via MCP (§3). As duas tabelas de nomes (`docs/keys.md [commands]` e
`nas-commands`) continuam independentes por agora; nada nesta etapa
renomeia uma pra bater com a outra.

**O objetivo é unificar os dois**, não deixá-los divergir pra sempre:
cada capacidade do nas vira **um** comando no registro
(`nas-commands`), do qual nascem, automaticamente, tanto o atalho pro
usuário (via `nas-keys`) quanto a ferramenta pra IA (via MCP, §3) — o
inverso de hoje, em que `nas-keys` despacha pra código Rust direto
(`machine.rs`/`App::apply_action`) sem passar pelo registro. Isso só
faz sentido **depois** que o núcleo em segundo plano (§2) e o MCP
(§3) já estiverem rodando de verdade e tiverem mostrado, na prática,
que forma de comando serve pros dois lados (a navegação por
caractere, por exemplo, não precisa de aprovação nem de esquema JSON
— só uma IA raramente chamaria "mover um caractere" como ferramenta
própria) — por isso não entra nas etapas 3a-3f. Fica registrada como
uma etapa própria, **3g — unificação do despacho de comandos**, a ser
detalhada (o que migra, o que nas-keys continua resolvendo sozinho)
quando a fase 3 chegar lá; até então, viver com os dois sistemas
separados é aceitável.

Os dez comandos iniciais (§3) e seu efeito, usando o que
`nas-editor::Document` já expõe:

| Comando | Efeito | Base em `Document`/`nas-history` |
|---|---|---|
| `open_file` | `ReadFile` | `Document::open` |
| `read` | `Pure` | `Document::full_text`/`line` (buffer já aberto, só memória) — com números de linha, §3 |
| `search` | `Pure` | `Document::find_from` (só memória) — resultados com números de linha, §3 |
| `list_files` | `ReadFile` | novo: lista os arquivos do projeto |
| `edit` | `WriteBuffer` | busca por texto exato + `Document::delete_region`/inserção — semântica detalhada em §3 |
| `show_diff` | `ReadFile` | `nas_history::diff` (lê `objects/` em disco) |
| `create_file` | `WriteFile` | novo: cria um arquivo vazio no disco (e abre como buffer) |
| `checkpoint_create` | `WriteHistory` | o checkpoint automático de uma sessão de IA (§6) reaproveita este mesmo comando |
| `checkpoint_restore` | `WriteFile` | `nas_history::restore` |
| `save_file` | `WriteFile` | grava no disco o que `edit` só mudou em memória — detalhado em §3 |

`edit` é `WriteBuffer`, não `WriteFile`: muda o texto em memória (como
qualquer tecla do usuário), não salva no disco por conta própria —
`Ctrl+s` continua sendo decisão do usuário, mesmo quando quem editou
foi a IA. Qual desses efeitos pede aprovação — e qual não — é decidido
pelo risco real de cada um, não só pela classe; ver §5.

### 2. Núcleo em segundo plano

**Revisão da ordem (§8): isto é a etapa 3b, agora depois do benchmark
(3f), não logo depois de 3a.** Até lá, `nas --mcp` (§3) mantém seu
próprio núcleo dentro do processo, igual ao `nas-term` — os dois não
compartilham memória nenhuma; uma mudança da IA só chega ao
`nas-term` pelo disco. O design abaixo é o que esta etapa implementa
quando (e se) a decisão pós-benchmark for seguir adiante.

O núcleo (conjunto de buffers abertos num projeto, o registro de
comandos, e o `Journal` do projeto) passa a poder viver **fora** do
processo do `nas-term`, pra que um agente funcione com a interface
fechada. Em vez de uma crate/binário novo, o próprio binário do
`nas-term` ganha um segundo modo de entrada: `nas --daemon` inicia só
o núcleo, sem terminal, escutando num socket Unix — reaproveita o
binário e o código que já existe, sem inflar o workspace com mais uma
crate antes de precisar.

- **Um núcleo por projeto**, identificado do mesmo jeito que o
  histórico (ADR 0005 §6: raiz do Git, ou o diretório onde foi
  iniciado). O socket vive em
  `$XDG_RUNTIME_DIR/nas/<hash-do-projeto>/core.sock` (mesmo fallback
  manual que `NAS_KEYBOARD_LAYOUT` e o `$XDG_STATE_HOME` do histórico
  já usam quando a variável não existe), com permissão **`0600`** no
  próprio arquivo do socket — não `~/.local/state`: aquele diretório é
  pra estado que sobrevive a reiniciar a máquina (journal,
  checkpoints); o socket é efêmero, do tempo de vida do daemon, e
  `$XDG_RUNTIME_DIR` já é o lugar certo pra isso (também seguro por
  padrão, geralmente `0700` do próprio usuário).
- **Autostart**: tanto o `nas-term` normal quanto o `nas-mcp` (§3)
  tentam conectar nesse socket primeiro; se não existir, sobem o
  próprio `nas --daemon` como processo filho antes de continuar. Quem
  abre o nas sem nenhum agente envolvido nunca percebe diferença —
  mesmo fluxo de hoje, sem socket nenhum, se preferir não usar IA (o
  autostart só acontece se algo tentar conectar).
- **Encerramento**: o daemon sai sozinho depois de um período
  (proponho 10 minutos) sem nenhum cliente conectado — checado por um
  intervalo periódico (`tokio::time::interval`), não fica pendurado
  pra sempre depois que o usuário fecha o `nas-term` e a sessão de IA
  termina.
- **Recuperação depois de uma queda**: se o processo do daemon morrer
  sem remover o arquivo do socket (um `kill -9`, um pane morto), o
  próximo cliente a tentar conectar recebe `ECONNREFUSED` (ninguém
  escutando um socket Unix cujo arquivo ainda existe, mas sem
  processo do outro lado) — nesse caso específico, remove o arquivo
  velho e tenta subir um daemon novo. Pra dois clientes não subirem
  dois daemons ao mesmo tempo nessa corrida, o autostart usa um
  arquivo de lock ao lado do socket (`core.sock.lock`, criado com
  `O_EXCL`): quem consegue criar o lock sobe o daemon; quem não
  consegue só espera um instante e tenta conectar de novo.
- **Vários clientes, um núcleo**: o núcleo serializa toda escrita (um
  só `Document::apply` por vez, como já é dentro de um processo hoje)
  e notifica todo cliente conectado quando algo muda — é assim que o
  `nas-term` aberto redesenha quando a IA edita um arquivo que o
  usuário está olhando, e é assim que dois agentes (ou um agente e o
  usuário) nunca pisam um no outro. Ver §2.1 para o caso de um
  `nas-term` e um agente terem o **mesmo** documento aberto ao mesmo
  tempo.
- Protocolo: mensagens JSON por linha sobre o socket (pedido/resposta
  simples, mais um canal de eventos de mudança) — não é um protocolo
  novo de propósito nenhum, só o suficiente pra ligar os dois lados;
  tanto o `nas-term` quanto o `nas-mcp` falam esse protocolo interno,
  nenhum dos dois expõe o socket diretamente pra fora.

#### 2.1 Edição simultânea do mesmo documento

Um `nas-term` aberto e um agente podem ter o mesmo arquivo aberto ao
mesmo tempo. Como os dois são clientes do **mesmo** `Document` dentro
do núcleo (não cada um com sua cópia local sincronizada depois), boa
parte do problema de "mesclar duas edições" nem chega a existir: não
há duas cópias pra reconciliar, há uma só, e cada transação (do
usuário ou da IA) se aplica direto nela, uma de cada vez, na ordem em
que o núcleo recebe.

- **Posição do cursor do usuário**: é estado só da UI (vive em
  `Buffer`/`App` do `nas-term`, nunca no `Document` compartilhado),
  então uma transação da IA nunca a move por si só — mas o *offset*
  numérico do cursor precisa ser remapeado através de toda transação
  aplicada por **qualquer** autor, do mesmo jeito que já é remapeado
  hoje através de um `undo`/`redo` locais. O núcleo guarda, por
  cliente `nas-term` conectado, o offset do cursor e da seleção
  daquele cliente, e os atualiza a cada transação usando a mesma
  função de mapeamento de posição que o `helix-core` já expõe
  (`ChangeSet`/`map_pos`) — nenhum algoritmo novo, só aplicá-lo também
  às transações de outros autores, não só às locais.
- **Alterações não salvas do usuário**: não existe um "local" separado
  do "remoto" pra reconciliar — a edição do usuário **é** a transação
  que chega ao `Document` compartilhado, no mesmo instante da tecla
  (o `nas-term` não guarda um rascunho próprio antes de enviar).
  `is_modified()` continua significando exatamente o que significa
  hoje — "o `Document` difere do que está no disco" — e vale pro
  documento como um todo, não por cliente.
- **`write_file`/`checkpoint_*` quando há alteração não salva do
  usuário**: como só existe um `Document`, salvar no disco salva
  **tudo** que está nele — a edição da IA e qualquer alteração do
  usuário que ainda não tinha ido pro disco, juntas, sem distinção.
  O pedido de aprovação (§5) avisa isso explicitamente quando for o
  caso — algo como "salvar `src/foo.rs` — inclui 12 linhas suas ainda
  não salvas" — pra nunca surpreender o usuário com o próprio trabalho
  sendo salvo sem ter pedido. Sem a aprovação, nada vai pro disco: nem
  a parte da IA, nem a do usuário (não existe "salvar só a parte de
  um autor" — é tudo o mesmo arquivo).
- **Ordem das transações**: estritamente a ordem de chegada ao
  núcleo (FIFO por documento, §2) — cada transação se aplica por
  completo antes da próxima começar, nunca duas em paralelo sobre o
  mesmo `Document`. O `edit` da IA (§3) ajuda bastante aqui por
  construção: como ele busca o texto antigo exato em vez de um
  intervalo de caracteres fixo, mesmo que o usuário tenha editado
  algo entre a última leitura da IA e este `edit`, a busca ainda acha
  o texto certo (se ainda existir) ou recusa com um erro claro (se já
  tiver sido alterado) — nunca aplica no deslocamento errado.
- **Desfazer é um histórico só, não um por autor**: `u` desfaz a
  **última transação do documento, de quem for** — do usuário ou da
  IA, pela mesma ordem FIFO acima — nunca só as próprias. A echo area
  diz de quem era a transação desfeita, ex. `"desfeito: edição da IA
  (claude-code)"` ou `"desfeito: sua edição"`, reaproveitando a
  autoria que o journal já guarda (ADR 0005 §1) — sem isso, desfazer
  algo que não foi você quem fez seria uma surpresa silenciosa. Pra
  desfazer o trabalho **inteiro** de uma sessão de IA de uma vez (em
  vez de tecla por tecla), vale o checkpoint automático da sessão
  (§6) — `Ctrl+t r` nele, não `u` repetido. **Desfazer seletivo por
  autor** (ex.: "desfaça só as edições da IA, mantendo as minhas que
  vieram depois, sem tocar nelas") fica como ideia pra depois, fora
  desta fase — exigiria recompor o texto a partir do journal pulando
  entradas de um autor específico, uma operação bem mais complexa que
  desfazer linear ou restaurar um checkpoint inteiro.
- **Teste de aceite (parte do critério de pronto de 3b)**: com um
  `Document` compartilhado, um cliente simulando o `nas-term` com o
  cursor no meio do texto e uma edição pendente de não-salvo, e um
  segundo cliente simulando a IA editando um trecho **antes** da
  posição do cursor — depois da transação da IA, o offset do cursor
  do primeiro cliente deve ter se deslocado exatamente pela diferença
  de tamanho introduzida, e `is_modified()` deve continuar `true`
  (nada se perdeu, nada precisou ser mesclado à mão).

### 3. Servidor MCP

Uma crate nova, `nas-mcp` — **biblioteca**, não um binário próprio
(revisão da etapa 3c: a entrada é `nas --mcp`, no mesmo binário que já
existe, não um segundo executável) — usando a crate
[`rmcp`](https://docs.rs/rmcp) isolada atrás do nosso próprio
adaptador, mesmo princípio já usado pro helix (decisão 0001: "isolar o
uso dessas crates atrás dos nossos módulos, a API delas não é
estável"). `nas-mcp` só sabe traduzir: uma chamada de ferramenta MCP
vira uma chamada ao `Registry` (§1); nada de `rmcp` aparece em
`nas-commands`, `nas-editor` ou `nas-history`.

- **Transporte**: stdio — o jeito que o Claude Code (e a maioria dos
  clientes MCP) espera, lançando `nas --mcp` como subprocesso.
  **Revisão da etapa 3c (§2/§8)**: até a decisão pós-benchmark sobre o
  núcleo em segundo plano (3b), não há socket nenhum pra conectar —
  `nas --mcp` mantém seu próprio `Workspace` dentro do processo, do
  mesmo jeito que `nas-term`. A frase original desta seção ("conecta
  no socket do núcleo do projeto corrente, subindo um núcleo novo se
  precisar") só passa a valer quando 3b existir.
- **Ferramentas**: exatamente os dez comandos de §1,
  `registry.specs()` filtrado por `exposed_to_ai` (cada `CommandSpec`
  marca explicitamente se aparece pra IA — estar no registro não
  basta, §5) e convertido 1:1 em `rmcp::Tool` (nome, descrição,
  `input_schema` = o `args_schema` do `schemars`) — a lista de
  ferramentas que a IA vê **é** o registro já filtrado; não existe uma
  segunda lista pra manter sincronizada.
- **`edit` é "texto antigo exato → texto novo", nunca intervalo de
  caracteres.** Argumentos: `{ path, old_text, new_text, line_range?:
  [de, até] }`. O comando busca `old_text` literal no conteúdo atual
  do buffer (não no que a IA leu por último — sempre o texto de
  agora, já que o núcleo é a única fonte de verdade, §2) e:
  - se não encontrar, recusa com um erro dizendo isso — nunca cria
    texto novo do nada;
  - se encontrar **mais de uma vez**, recusa pedindo pra desambiguar
    com `line_range` (um intervalo de linhas que contém a ocorrência
    certa) — nunca adivinha qual;
  - se encontrar exatamente uma vez (direto, ou já restrito por
    `line_range`), substitui por `new_text`.

  Isso resolve por construção boa parte do risco de edição
  simultânea (§2.1): como o `edit` nunca depende de um deslocamento
  numérico calculado antes, um documento que mudou entre a última
  leitura da IA e este `edit` não faz a edição cair no lugar errado
  — ou o texto antigo ainda está lá (edita certo) ou não está mais
  (recusa, nunca corrompe outro trecho).
- **`edit` muda só o buffer do processo MCP; nada vai pro disco sem
  `save_file`** (revisão da etapa 3c). Até lá, o arquivo no disco e o
  que a IA vê podem divergir — por isso toda leitura/edição começa
  reaproveitando a mesma detecção de mudança externa do `Document`
  (ADR 0005 §1): se o buffer da IA está limpo e o disco mudou,
  recarrega sozinho antes de continuar; se o buffer tem alterações e o
  disco mudou, avisa no resultado do comando em vez de recarregar (nunca
  descarta trabalho silenciosamente). `save_file` (`WriteFile`, pede
  aprovação — abaixo) grava o buffer no disco, confere o caminho
  contra a raiz do projeto e recusa — com um erro pedindo pra reabrir
  via `open_file` — se o disco mudou desde a última leitura; nunca
  sobrescreve o trabalho de outro processo (o `nas-term` do usuário,
  por exemplo).
- **`create_file`**: cria um arquivo novo (vazio, ou com um conteúdo
  inicial opcional) e já abre como buffer — `WriteFile`, pede
  aprovação (§5).
- **`list_files`**: lista os caminhos do projeto (respeitando
  `.gitignore`, como os outros comandos que já leem do projeto) — sem
  conteúdo, só os caminhos.
- **Toda leitura devolve texto com números de linha** — `read`,
  `search` (cada resultado com a linha onde apareceu) e `list_files`
  onde aplicável. É o que permite à IA montar o `line_range` de um
  `edit` subsequente sem precisar contar linhas por conta própria.

### 4. ACP

**Não entra nesta fase.** ACP (Agent Client Protocol, da Zed) é a
direção contrária do MCP: em vez de "o agente chama ferramentas do
nas", é "o nas embute a sessão de um agente e mostra o que ele propõe
com UI própria" — o pilar 4 do README ("a IA propõe uma frase de
comandos... o usuário lê antes de executar", estilo Sequential) é
exatamente isso, só que hoje o Claude Code já tem sua própria UI de
terminal pra mostrar diffs e pedir aprovação. MCP já resolve o
objetivo central desta fase — um agente de verdade operando o nas,
testável com o benchmark do §7 — sem exigir que o `nas-term` vire um
cliente ACP. ACP é jovem (como o `rmcp`, cautela parecida à da decisão
0001) e é trabalho de UI ortogonal ao núcleo/registro/permissões desta
fase. Fica pra uma ADR própria, depois que o uso real via MCP mostrar
se vale a pena uma UI nativa de sessão dentro do nas-term.

### 5. Permissões pelo risco real

Refino o pilar 2 do README: em vez da mesma barreira pra todo efeito
não-`Pure`, a aprovação é por **risco real**, não só pela classe —
e só quando o autor **não** é `Author::User` (um humano nunca precisa
aprovar a própria tecla). **Revisão da etapa 3d**: a checagem vale
pra todo autor que não seja o usuário, não só `Author::Ai` — uma
extensão futura (`Author::Extension`) ou uma mudança detectada por
fora (`Author::External`) nascem bloqueadas por padrão igual a um
agente de IA, em vez de precisarem de uma exceção própria no dia em
que algo de verdade as produzir.

- **Liberados por padrão**: `Pure`, `ReadFile`, **`WriteBuffer`** e
  **`WriteHistory`**. `WriteBuffer` entra aqui de propósito — é
  reversível pelo histórico (`u`, journal) e, numa sessão de IA, já
  está coberto pelo checkpoint automático do início da sessão (§6):
  desfazer qualquer coisa que `edit` fez é sempre possível, com ou
  sem aprovação prévia. Pedir aprovação pra cada edição de buffer
  tornaria o fluxo comum (abrir, ler, editar, repetir) tão lento
  quanto sem o nas — o próprio propósito do benchmark do §7.
  `WriteHistory` entra pelo mesmo motivo que `WriteBuffer`, por um
  caminho diferente: `checkpoint_create` só acrescenta um snapshot ao
  armazenamento do histórico (`~/.local/state/nas/`) — nunca toca o
  arquivo do projeto, então não há nada ali pro usuário aprovar
  contra; é puro ganho de segurança (mais um ponto pra restaurar),
  nunca um risco.
- **Pedem aprovação**: `WriteFile`, `Shell` e `Network` — tocam o
  disco de verdade (fora do armazenamento do histórico) ou saem do
  projeto. Isso cobre `create_file`, `checkpoint_restore` e
  `save_file` entre os comandos de §1 — `checkpoint_create` não,
  pela razão acima.
  **Efeito dependente de argumento (revisão da etapa 3f-prep, §8):**
  `edit` ganhou uma opção `save` que grava a mudança no disco na
  mesma chamada — o efeito de uma chamada de `edit` passa a ser
  `WriteBuffer` (livre) ou `WriteFile` (pede aprovação), decidido
  pelo valor de `save` **daquela chamada**, nunca um efeito fixo por
  comando. `CommandSpec` passa a distinguir dois efeitos: o nominal
  (`effect`, o que a maioria dos comandos usa sempre) e o "pior caso"
  entre todos os argumentos possíveis (`max_effect`) — só este
  segundo é usado pra anotar a ferramenta em `tools/list`
  (`destructive_hint`, abaixo), de forma conservadora, já que a lista
  de ferramentas não vê os argumentos de uma chamada específica
  ainda. A decisão de aprovação de verdade nunca usa nenhum dos dois
  direto — usa sempre `effective_effect(args)`, calculado a partir
  dos argumentos crus da chamada (uma leitura otimista de `args.save`,
  nunca uma validação completa — essa continua sendo o `Deserialize`
  de dentro do próprio comando). Esse cálculo mora no `Registry::run`
  em si (não só em `nas-mcp`), que é o único caminho de execução de
  qualquer comando — por isso não existe jeito de `edit(save: true)`
  gravar no disco sem passar pela aprovação de `write-file`, mesmo
  chamando o registro direto (como um teste faz), sem o adaptador MCP
  no meio.
  **`--approve-writes` (revisão da etapa 3c, temporário)**: até a
  etapa 3d existir de verdade, `nas --mcp --approve-writes` foi o
  único jeito de liberar `WriteFile` (nunca `Shell`/`Network`, que não
  têm comando nenhum ainda) — pensada só pra testar o resto do fluxo
  (`save_file` incluído) antes de 3d existir. **Removida na própria
  etapa 3d** (ver abaixo), substituída pelo arquivo de política —
  nunca conviveu com ele como uma segunda porta.
- **Interface aberta — revisão da etapa 3d: passa pra 3b.** A ideia
  original abaixo (pedido bloqueante com echo area, "uma vez"/"sempre
  na sessão") depende do `nas-term` e do processo que roda os
  comandos da IA estarem **conectados** — hoje (3c/3d, sem núcleo em
  segundo plano) não estão: cada um mantém seu próprio `Workspace` em
  processo, sem nada pra notificar. Essa conexão só passa a existir
  com o daemon (§2, etapa 3b) — é lá que esta interface entra, não em
  3d. O texto abaixo é o design que 3b implementa quando chegar lá,
  não o que 3d entrega:
  o núcleo manda um pedido de permissão pra todo cliente `nas-term`
  conectado e **bloqueia** aquele comando até decidir (com um tempo
  limite — alguns minutos — que nega automaticamente; nunca aplica por
  silêncio, nunca trava o agente pra sempre). `nas-term` mostra isso na
  echo area, reaproveitando o mesmo mecanismo which-key da paleta de
  comandos: um título com o agente e o comando, e as opções **uma
  vez**, **sempre na sessão** e qualquer outra tecla cancela (nega) —
  mesmo padrão de "qualquer tecla fecha" que já existe pros overlays.
  "Sempre na sessão" vale só pelo resto daquela sessão daquele agente
  (o par `agent`+`session` de `Author::Ai`, §6) — nunca é escrito em
  disco automaticamente; uma nova sessão de IA sempre pergunta de novo.
- **Interface fechada — o que a etapa 3d de fato implementa.** Sem
  núcleo em segundo plano (3b não existe ainda) e sem ninguém do
  `nas-term` pra perguntar, 3d entrega só o arquivo: `nas --mcp`
  consulta um arquivo de configuração por projeto
  (`~/.local/state/nas/history/<hash>/permissions.toml` — ao lado do
  `journal.jsonl` e dos `checkpoints/` já existentes da ADR 0005,
  **não** em `$XDG_RUNTIME_DIR`: é configuração que precisa sobreviver
  a reiniciar a máquina, diferente do socket do núcleo, §2), que o
  usuário mantém manualmente de antemão (`nas --permissions` mostra o
  caminho e cria um modelo comentado, com tudo negado, se ainda não
  existir), liberando os efeitos que confia rodar sem supervisão
  (`write-file`, `shell`, `network`, algum ou nenhum).
  Um efeito sem entrada nesse arquivo é **negado direto**, com um erro
  claro de volta pro agente dizendo o que falta e como liberar (`nas
  --permissions`) — nunca aplica por padrão, nunca finge sucesso. Um
  arquivo inválido nega **tudo**, com o motivo na mensagem (mesma
  ideia de configuração recuperável da ADR 0007 §3). Cada pedido —
  negado ou aprovado — é registrado em
  `~/.local/state/nas/history/<hash>/permissions_log.jsonl` (quem
  pediu, o comando, o efeito, a decisão e, numa aprovação, se veio do
  arquivo, de uma pergunta ou de "sempre na sessão" — hoje sempre
  "arquivo", única fonte que existe).
  **A pergunta interativa (item (b) do plano original desta etapa) e
  "sempre na sessão" ficam de fora de 3d, de propósito**: ambas
  dependem de `elicitation` do MCP, e o `rmcp` fixado neste workspace
  (`0.3`, ver `Cargo.toml` raiz) não suporta `elicitation` — o recurso
  só existe em versões bem posteriores (major `1.0+`) da própria
  crate. O cliente (Claude Code) já suporta desde a versão `2.1.76`;
  o bloqueio é só do lado da biblioteca Rust. Confirmado antes de
  implementar, como pedido: nenhuma das duas partes (arquivo de
  política sozinho, ou arquivo + pergunta) ficou pra trás por decisão
  de projeto — é a biblioteca pinada que não dá. Migrar `rmcp` pra uma
  versão com suporte é uma mudança grande (salto de major version,
  API bem diferente da 0.3) — fica registrada como etapa **3h** (§8),
  depois da decisão do benchmark, não parte de 3d.
- **O benchmark do §7 usa uma política explícita**: pra rodar as
  tarefas sem travar em cada aprovação, o arquivo de configuração do
  benchmark libera exatamente os efeitos que as tarefas escolhidas
  precisam (tipicamente nenhum `Shell`/`Network`, já que são tarefas
  de edição de código) — essa política fica documentada dentro do
  próprio `docs/benchmark-fase3.md` (§7), não escondida numa
  configuração solta.

### 6. Autoria

Nada muda no tipo (`Author::Ai { agent, session }` já existe e já é
testado, ADR 0005 §1) — só passa a ser preenchido de verdade: `agent`
vem do *handshake* de inicialização do MCP (o nome que o cliente se
declara, com um padrão de configuração se o cliente não declarar
nada); `session` é gerado uma vez por conexão ao núcleo.

**Revisar e desfazer uma sessão inteira não precisa de mecanismo
novo (implementado na etapa 3e).** Na primeira chamada efetiva de um
`(agent, session)` — um comando com efeito `WriteBuffer`/`WriteFile`
que de fato muda algo, nunca uma tentativa que falhou —
`Registry::run` cria um checkpoint automático **antes** de aplicar a
mudança, chamado `"antes da sessão de <agent> (<data e hora>)"`, com
o estado de todo arquivo já rastreado (buffer aberto, se houver;
senão o disco) — o mesmo padrão que a restauração já usa pra se
tornar desfazível (ADR 0005 §7). Um só por `(agent, session)`, nunca
repetido — chamadas seguintes da mesma sessão não criam outro. `Ctrl+t
l`, no `nas-term`, já identifica o autor de cada checkpoint na própria
linha (`você`/`IA (agente)`/...), não só pelo nome.

Depois disso:

- `Ctrl+t a` (autoria) já mostra, linha a linha, o que foi daquela
  sessão — sem duplicar como `Author::External` quando a IA salva
  (issue #28, abaixo).
- `Ctrl+t e` entre o checkpoint automático e agora mostra o diff
  inteiro da sessão.
- `Ctrl+t r` nesse mesmo checkpoint desfaz a sessão inteira de uma vez
  — a mesma composição que ADR 0005 §7 já usa ("desfazer uma
  restauração é só restaurar o checkpoint automático que ela mesma
  criou"), reaproveitando `nas_history::restore` sem nenhuma mudança:
  o checkpoint de sessão é um checkpoint automático como qualquer
  outro pra esse algoritmo.

Nenhuma dessas três teclas precisa saber que existe um agente — elas
já tratam `Author::Ai` igual a qualquer outro autor.

**Restaurar o checkpoint de sessão volta o projeto inteiro àquele
momento — não só os arquivos que a sessão tocou.** `nas_history::restore`
já funciona assim pra qualquer checkpoint (docs/decisions/0005 §4): tudo
que foi tocado por qualquer autor, depois do checkpoint, é revertido ao
original ou jogado na lixeira (nunca apagado de vez), mesmo que não
tenha sido a sessão quem tocou. Na prática, isso inclui edições suas
feitas durante a sessão, em arquivos que a IA nunca abriu — **elas não
se perdem**: o próprio `restore` cria, antes de mexer em qualquer
coisa, o checkpoint automático "antes de restaurar", com o estado de
tudo naquele instante (a sua edição incluída). Desfazer *seletivamente*
só a parte da IA, preservando edições suas feitas depois sem tocar
nelas, é trabalho da futura etapa de revisão (§8) — não desta. Mesmo
aviso em `docs/mcp.md`.

**Issue #28 (sem autoria duplicada):** uma edição salva pela IA
(`save_file`/`create_file`/restaurar um checkpoint, via `nas --mcp`)
não pode aparecer duas vezes no journal — uma como `Author::Ai`, no
momento do `save`, e outra como `Author::External` quando um
`nas-term` aberto no mesmo arquivo detecta a escrita no disco (ADR
0005 §1). A correção: toda escrita de verdade em disco
(`Workspace::record_last_save`) registra o hash do conteúdo gravado,
por autor, num registro compartilhado (`nas_history::record_last_save`/
`get_last_save`, sobrescrito a cada gravação — "a última", não "a
primeira", diferente de `originals`). Ao detectar uma mudança
externa, `nas-term` confere esse registro antes de journalar: bate o
hash? então já foi journalada por outro autor, e `nas-term` só
recarrega/avisa, sem gravar uma segunda entrada. Mudanças externas de
verdade (outro programa, `git checkout`, etc.) continuam registradas
como `Author::External` normalmente — nada muda pra elas.

**Issue #24 (sem falha em silêncio):** `ensure_tracked` e o flush do
journal usavam `let _ =` em três lugares (`nas-term::app`,
`nas-commands::context`, `nas-commands::registry`) — uma falha ali
nunca impedia o comando (não deveria: o texto já foi editado/salvo de
verdade), mas também não podia desaparecer: sem o original guardado,
um checkpoint futuro (nomeado, automático, ou o de sessão desta
etapa) não consegue restaurar aquele arquivo de volta. Agora: no
`nas-term`, aparece na echo area; no MCP, `Registry::run` mescla o
aviso no resultado da própria ferramenta (campo `warning`, o mesmo
canal que já existia pra avisos de sincronização com o disco) — nunca
um erro que recuse o comando (o efeito real já aconteceu), mas nunca
descartado em silêncio tampouco.

### 7. Teste comparativo

**O relatório precisa registrar a ressalva de 3c** (§2/§8): até a
decisão pós-benchmark sobre o núcleo em segundo plano (3b), `nas-mcp`
não compartilha buffer com um `nas-term` eventualmente aberto no
mesmo arquivo — a sincronização é só por disco. Qualquer lentidão ou
atraso em perceber uma mudança observado durante o benchmark deve ser
atribuído a essa limitação conhecida da etapa, nunca lido como um
problema da integração MCP em si.

**Hipótese**: o nas oferece mais rastreabilidade, controle e
reversibilidade, mantendo eficiência e custo competitivos em relação
ao uso direto do agente. Não é "o nas é mais rápido ou mais barato" —
é que o ganho em controle/reversibilidade que só o nas oferece
(checkpoint automático de sessão, diff e desfazer por autor, §6) vale
a pena mesmo sem perder em eficiência de forma relevante. O critério
de decisão abaixo fixa o que conta como "relevante" antes de rodar.

**A comparação é nas contra o Claude Code no seu modo normal** — com
as próprias ferramentas de ler, editar e buscar, além do shell — não
contra "só shell". É a comparação justa: o Claude Code já lê/edita
arquivos e roda comandos muito bem sem o nas; a pergunta que importa é
se o `nas-mcp` (leitura com números de linha, `edit` por texto exato,
checkpoint automático de sessão, §1/§3/§6) melhora sobre o que o
Claude Code já faz por conta própria, não se "uma ferramenta" é melhor
que "nenhuma".

Cinco tarefas pequenas e reais, tiradas do backlog atual do próprio
nas (não sintéticas) — por exemplo: corrigir um bug concreto já
conhecido, adicionar um teste faltando pra uma função existente,
renomear algo de forma consistente numa crate, explicar o que uma
função faz, extrair uma pequena duplicação. Cada tarefa roda **três
vezes em cada lado** (seis rodadas por tarefa) — mesmo modelo, mesmo
pedido (o mesmo texto de prompt nas seis rodadas da mesma tarefa), em
**ordem alternada** (lado A, lado B, lado A, lado B, ... tarefa a
tarefa, não as três do mesmo lado em sequência) — pra não deixar
aquecimento de contexto ou ordem de execução favorecer um lado só
porque rodou primeiro ou por último.

**Condições**, fixadas pra que a comparação seja justa nos dois
lados:

- mesmo modelo, mesma versão do modelo e as mesmas cinco tarefas
  (mesmo texto de prompt) nos dois lados;
- no lado nas, as ferramentas de edição/leitura próprias do agente
  (Read/Edit/Write nativas do Claude Code) ficam **desligadas** — a
  única forma de tocar o arquivo é pelas ferramentas do `nas-mcp`
  (§3); sem isso, o agente poderia contornar o nas e a comparação não
  testaria nada;
- **coleta automatizada**, não transcrita à mão — por exemplo
  `claude -p --output-format json`, que já devolve tokens, custo e
  chamadas de ferramenta por sessão, reduzindo erro de transcrição e
  viés de quem está rodando o benchmark.

**Métricas por execução** (mesma unidade nos dois lados, direto
comparáveis):

- tokens de entrada e tokens de saída;
- custo (em dólares, pelo preço do modelo usado);
- chamadas de ferramenta (contagem);
- tempo de parede;
- **sucesso**: testes do nas passando depois da mudança (`cargo test
  --workspace`), não só revisão humana — as cinco tarefas são
  escolhidas, de propósito, entre as que admitem essa verificação
  objetiva;
- **retrabalho**: edições recusadas (ex.: `edit` que não achou o
  texto antigo, ou achou mais de uma vez), correções (uma segunda
  tentativa pra mesma mudança) e releituras (ler de novo um trecho já
  lido antes, nas ferramentas de qualquer um dos lados).

**Integridade e rastreabilidade** ficam num quadro separado, não
misturadas às métricas acima: o lado sem nas não tem o recurso
equivalente, então não é uma comparação simétrica como as de cima —
é um registro só do lado nas, documentando o que o benchmark não
precisaria "vencer" pra já valer, por não ter contraparte do outro
lado. Reaproveita a mesma medida que já estava aqui: depois de cada
rodada, quanto trabalho dá pra revisar a mudança e desfazê-la por
completo — do lado do nas, `Ctrl+t e`/`Ctrl+t r` no checkpoint
automático da sessão (§6), medido em teclas/comandos; do lado do
Claude Code normal, o que ele mesmo oferece pra isso (diff de sessão
próprio, `git diff`/`git checkout` manual, o que for o caminho real,
anotado como referência, não como "perda" dele no placar).

**Critério de decisão, fixado antes de rodar** (pra não racionalizar
o resultado depois de ver os números): taxa de sucesso igual ou maior
que o lado sem nas, **e** no máximo 25% a mais de tokens no total
(entrada + saída, somadas as três rodadas de cada tarefa) do lado
nas. Passou
nos dois: o nas segue como editor, fase 3 continua pelas etapas
seguintes (§8). Falhou em qualquer um dos dois: pivô — o nas deixa de
perseguir ser um editor de propósito geral competitivo em eficiência
de edição, e passa a focar em histórico e supervisão sobre o que a IA
já faz direto, em vez de MCP de edição.

Registro: uma tabela em `docs/benchmark-fase3.md`, uma linha por
(tarefa, lado, rodada) — trinta linhas no total (cinco tarefas × dois
lados × três rodadas) — mais o quadro separado de integridade e
rastreabilidade acima — rodada manual registrada à mão (com coleta
automatizada por sessão, como descrito nas condições), não um
benchmark automatizado de CI; o objetivo é um retrato antes de
investir mais, não uma suíte de regressão. A política de permissão
usada durante o benchmark (§5) fica documentada no topo do próprio
`docs/benchmark-fase3.md`.

### 8. Etapas

Divisão em PRs pequenos, mesmo espírito da fase 2 (2a–2d). **Os
nomes (3a, 3b, ...) identificam a etapa, não a ordem de execução** —
a ordem real, revisada, é **3a → 3c → paleta de comandos (fora da
fase 3) → 3d → 3e → revisão → 3f-prep → 3f (benchmark) → decisão →
3h (rmcp 1.x + elicitation) → 3b (núcleo em segundo plano) → 3g**,
com janelas e modo de notas — também fora da fase 3 — só depois do
ponto de decisão (ver abaixo).
O daemon (3b) virou a penúltima, não a segunda: é a etapa mais cara
desta fase
(processo novo, protocolo entre processos, autostart, remapeamento
de cursor entre clientes) e nada que o benchmark do §7 mede depende
dela — `nas-mcp` já funciona sem ela (ver a ressalva em 3c, abaixo).
Melhor descobrir com o benchmark se este caminho (registro + MCP)
vale a pena *antes* de pagar o custo do núcleo em segundo plano, não
depois.

- **3a** (pronta) — `nas-commands`: `Effect`, `CommandSpec`/
  `Registry`, `schemars` para o esquema, os comandos do registro
  (§1/§3) chamando direto um `Document`/`Journal` em processo (sem
  núcleo em segundo plano ainda) — testável isolado.
- **3c** — `nas-mcp`: adaptador `rmcp` sobre os comandos do registro
  (incluindo a semântica de `edit` por texto exato, §3), transporte
  stdio, `nas --mcp`. **Sem o núcleo em segundo plano (3b) ainda**:
  `nas --mcp` mantém seu próprio núcleo dentro do processo, do mesmo
  jeito que `nas-term` mantém o dele — os dois não compartilham
  memória. Uma mudança da IA chega ao `nas-term` (se ele estiver
  aberto no mesmo arquivo) só pelo disco, pela mesma detecção de
  mudança externa que já existe (ADR 0005 §1) — nunca ao vivo, num
  buffer compartilhado. **Isto precisa constar no relatório do
  benchmark (3f)**: uma eventual lentidão ou atraso em perceber a
  mudança é uma limitação desta etapa, não da integração MCP em si —
  sem essa ressalva, o benchmark mediria o núcleo que ainda não
  existe, não o que 3c de fato entrega.
- **paleta de comandos** (fora da fase 3) — item do roadmap mais
  amplo do nas, não desta ADR (`Shift+Espaço`, já reservado em
  `docs/keys.md`, depende do protocolo de teclado do Kitty). Entra
  nesta lista só pra marcar a ordem real entre entregas do projeto —
  fica entre 3c e 3d no calendário, sem ganhar uma etapa "3-algo"
  nova nem escopo detalhado aqui.
- **3d** (pronta) — permissões pelo risco real: o bloqueio pra
  `WriteFile`/`Shell`/`Network`, feito dentro do próprio processo
  `nas --mcp` (síncrono com a chamada MCP, sem núcleo em segundo plano
  ainda — mesma ressalva de 3c). Entrega **(a)** o arquivo de política
  por projeto (`permissions.toml`, `nas --permissions`) e o registro
  de cada decisão (`permissions_log.jsonl`) — §5. **Não entrega (b)**
  a pergunta interativa via `elicitation`, nem "sempre na sessão": o
  `rmcp` fixado neste workspace (`0.3`) não suporta `elicitation`
  ainda (confirmado antes de implementar, conforme pedido) — fica
  registrado em §5 como trabalho futuro, junto da migração de versão
  que destravaria isso. O prompt na echo area do `nas-term` (que
  dependia de nas-term/MCP conectados) **passa pra 3b** — não é mais
  parte de 3d. **Remove `--approve-writes`** (a flag temporária de 3c,
  §5), substituída pelo arquivo de política — a flag não sobrevive a
  esta etapa.
- **3e** (pronta) — autoria de sessão: o checkpoint automático por
  `(agent, session)` (§6), um teste de ponta a ponta provando que
  `Ctrl+t r` nesse checkpoint desfaz a sessão inteira (reverte o que
  já existia, joga na lixeira o que a sessão criou), sem perder uma
  edição concorrente do usuário (recuperável pelo "antes de
  restaurar"). Também corrige as issues #24 (falha de `ensure_tracked`/
  flush do journal não pode passar em silêncio) e #28 (edição salva
  pela IA não pode aparecer duplicada como `Author::External`) — as
  duas ficaram pendentes pra esta etapa porque dependiam da autoria de
  sessão estar implementada pra fazer sentido testar de ponta a ponta.
- **revisão** — uma tela pra revisar o trabalho de uma sessão de IA
  (ou de um período) antes de confiar nele, trecho a trecho:
  - um buffer único com todos os trechos alterados, de todos os
    arquivos tocados na sessão (ou no período escolhido), cada um com
    antes/depois, arquivo, linha e autor;
  - navegação trecho a trecho, com três ações por trecho: aceitar
    (marca como revisado), rejeitar (desfaz só aquele trecho —
    recusa, com aviso, se o trecho mudou depois da edição da IA, do
    mesmo jeito que `edit` já recusa uma edição sobre texto que não é
    mais o que a IA viu, §3) e ir até o arquivo, naquela linha;
  - "antes" vem do checkpoint automático da sessão (3e); a autoria,
    do journal (ADR 0005 §1) — nada novo pra guardar, só exibição
    sobre o que já existe. Comparar contra o último commit do Git,
    em vez do checkpoint da sessão, fica como base opcional, pra
    revisar tudo que mudou desde o último commit, não só desde a
    sessão atual.
  - diff unificado primeiro (não depende de janelas, §8); lado a
    lado fica pra depois, quando o nas tiver janelas (fora da fase
    3, só depois da decisão, acima).

  **Motivo**: vale nos dois desfechos do benchmark (3f) — se passar,
  é parte do produto que a fase 3 está construindo; se falhar e o
  foco pivotar pra histórico e supervisão em vez de MCP de edição,
  a revisão **é** esse produto, não trabalho perdido. Também dá uma
  métrica de controle que o §7 ainda não media: quanto tempo leva
  pra revisar o trabalho de uma sessão de IA inteira, não só pra
  desfazê-la.
- **3f-prep** — eficiência das ferramentas do MCP (§3): ajustes nas
  ferramentas existentes antes de medi-las no benchmark (3f), porque
  cada ferramenta nova ocupa contexto em toda mensagem — por isso o
  princípio desta etapa é **preferir parâmetro novo a ferramenta
  nova**, sempre que der. Escopo fechado:
  - `read` ganha um intervalo de linhas opcional, em vez de sempre
    devolver o arquivo inteiro;
  - `edit` ganha uma opção pra já salvar no disco depois de editar,
    sem precisar de uma segunda chamada a `save_file`;
  - `read`, `edit` e `save_file` abrem o arquivo por conta própria se
    ele ainda não estiver aberto como buffer — hoje dependem de um
    `open_file` anterior;
  - `edit` devolve, no resultado, o trecho alterado com algumas
    linhas de contexto ao redor, em vez de só confirmar sucesso — pra
    a IA conferir o efeito sem precisar de um `read` extra;
  - duas ferramentas novas — única exceção ao princípio acima, por
    não existir parâmetro equivalente nas ferramentas de hoje:
    `list_symbols` e `read_symbol`, via Tree-sitter, começando por
    Rust e títulos de Markdown (não todas as linguagens de uma vez).

  **Implementado (revisão): `tree-house-bindings`, não a crate
  oficial `tree-sitter`.** O plano original era a crate oficial
  `tree-sitter` + `tree-sitter-rust`, com a gramática embutida no
  binário via `cc` (crate nova, `nas-syntax`, isolando Tree-sitter do
  resto do workspace do mesmo jeito que a decisão 0001 isola
  `helix-core`). Na prática, isso **não linka**: `helix-core` (já
  fixado por aquela mesma decisão 0001) depende de
  `tree-house-bindings` — o fork do Helix das próprias bindings de
  tree-sitter, que vendora e compila sua própria cópia do runtime C
  (`language.c`, `lib.c`, ...). A crate oficial `tree-sitter` vendora
  e compila **a mesma coisa**, com os mesmos nomes de símbolo C (sem
  *mangling*, ex. `ts_language_delete`), então o binário `nas`
  (que precisa das duas metades — `nas-editor`/`helix-core` e
  `nas-commands`/`nas-syntax`) falha o link com dúzias de `duplicate
  symbol`. Não existe um jeito de pedir pra nenhuma das duas crates
  não vendorar sua própria cópia.

  A correção, isolada inteiramente dentro de `nas-syntax` (nenhum
  outro crate sabe que `tree-house-bindings` existe): depender da
  **mesma** `tree-house-bindings` que `helix-core` já traz — o Cargo
  unifica pra uma única cópia compilada quando a versão é
  compatível, então não há nenhuma duplicação. Antes de usar essa
  crate de baixo nível, de fato avaliada a camada de cima que o
  Helix usa pra destaque de sintaxe (`tree-house`, que a reaproveita
  por baixo) — ela não serve pro que `list_symbols`/`read_symbol`
  precisam: sua API (`Syntax::new`) exige um `LanguageLoader`
  fornecendo `LanguageConfig` com *queries* `.scm` compiladas
  (highlights/injections/locals), pensada pra destaque incremental
  com injeção de linguagens, não pra "percorrer a árvore de um
  arquivo e extrair os nomes com seus campos" — a montagem desse
  aparato só pra satisfazer o contrato, sem usar nada do que ele
  oferece de fato, seria complexidade sem benefício agora (ela
  continua candidata óbvia pro destaque de sintaxe de verdade,
  quando essa etapa chegar — `nas-syntax` não fica menos reaproveitável
  por isso, já que é a mesma base). A API de baixo nível
  (`Parser`/`Tree`/`Node`/`TreeCursor`), em troca, é próxima o
  suficiente da API da crate oficial (que o protótipo original já
  usava) pra ser uma troca direta — a diferença principal é que o
  parser precisa de um `Input`/`Cursor` (do `regex-cursor`, pensado
  pra rope) em vez de aceitar um `&str` puro; como não há
  fragmentação nenhuma pra percorrer (o arquivo inteiro cabe num `&str`
  contíguo), a implementação é um `Input` trivial de um único
  "chunk" (`nas-syntax/src/rust.rs`, `StrInput`). Compatibilidade de
  versão (ABI) confirmada na prática, não só por leitura: os testes
  deste crate (`cargo test -p nas-syntax`) de fato chamam
  `Grammar::try_from(tree_sitter_rust::LANGUAGE)` e falhariam com
  `IncompatibleVersion` se a gramática gerada por `tree-sitter-rust
  0.24.2` (ABI 14 ou 15) não estivesse dentro da faixa que o
  `tree-house-bindings` fixado aceita (`13..=15`); passam. Markdown
  não usa Tree-sitter nenhum (títulos ATX são reconhecíveis por
  escaneamento de texto puro, `nas-syntax/src/markdown.rs`), então não
  há gramática de Markdown cuja compatibilidade de ABI precisasse ser
  conferida.

  **Tamanho de `tools/list` (pedido no resumo do PR), antes e depois
  desta etapa** (medido chamando `tools/list` de verdade sobre o
  protocolo, `docs/mcp.md`): 10 ferramentas, 4541 caracteres
  (~1135 tokens estimados, 4 caracteres/token) antes; 12 ferramentas,
  **4849 caracteres (~1212 tokens estimados) depois** — +7%, não os
  quase 60% que a primeira versão desta etapa chegou a medir (7187
  caracteres) antes da revisão abaixo.

  **Revisão de PR: as descrições inflavam `tools/list` com detalhe
  de implementação, não de uso.** O `schemars` usa, por padrão, o
  próprio doc comment Rust (`///`) de cada campo como a descrição no
  esquema JSON — e esses comentários foram escritos pra quem lê o
  código (referências de ADR, nomes internos como `DiskChanged`/
  `WriteBuffer`, por que uma regra existe), não pra quem só precisa
  saber como chamar a ferramenta. Isso vai pro modelo em toda
  mensagem de uma sessão, então pesa de verdade na métrica de tokens
  do benchmark (3f) — o `tools/list` em si, não só o custo por
  chamada. Correção, em três partes:
  - cada campo que precisa de explicação ganha
    `#[schemars(description = "...")]` — uma frase curta, escrita
    pra quem chama (ex.: `save: "grava no disco após editar; pede
    permissão de escrita"`) — que o `schemars` sempre prioriza sobre
    o doc comment quando os dois existem; o comentário de
    implementação continua no código, como `//` comum, fora do
    esquema; um campo autoevidente (`path`, na maioria dos comandos)
    não ganha descrição nenhuma, pra não repetir a mesma frase doze
    vezes;
  - a descrição de cada um dos doze comandos (o segundo argumento de
    `typed_command`) virou uma frase objetiva só, sem referência
    interna (nada de "ADR 0006", `CommandError::...`,
    `Workspace::...`);
  - `nas-mcp::spec_to_tool` remove `$schema` (a URL do meta-esquema)
    e `title` (o nome da struct Rust, ex. `"EditArgs"`) do
    `inputSchema` de cada ferramenta — nenhum dos dois ajuda o
    agente a chamar a ferramenta, só repete texto fixo doze vezes.

  Um teste em `nas-mcp` (`tools_list_cabe_no_orcamento_de_tamanho`)
  monta a mesma lista que `tools/list` devolveria e falha se passar
  de 5500 caracteres — pra uma ferramenta nova, ou uma descrição que
  cresça demais depois, não regredir esse número em silêncio.

  O ganho do resto da etapa (intervalo de `read`, `save` de `edit`,
  abrir sozinho, trecho de contexto no `edit`) não aparece em
  `tools/list` — é por chamada, no benchmark (3f): menos `open_file`
  redundante, menos `read` de arquivo inteiro só pra achar um
  trecho, menos `save_file` como segunda chamada, menos `read` de
  confirmação depois de um `edit`.
- **3f** — benchmark comparativo: rodar as cinco tarefas, três vezes
  por lado, em ordem alternada (§7), escrever
  `docs/benchmark-fase3.md` — **incluindo a ressalva de 3c** sobre a
  ausência de buffer compartilhado.
- **Decisão** — com o benchmark em mãos, decidir se vale seguir pra
  3b (o núcleo compartilhado) ou se o caminho de 3c-3e já basta por
  agora. Não é uma etapa de código; é o ponto em que a fase 3 para e
  reavalia, com dados, antes do passo mais caro. **Janelas e modo de
  notas** — ambos fora da fase 3 e sem ADR própria ainda — só entram
  no roadmap depois deste ponto, nunca antes: nenhum dos dois ajuda a
  responder a pergunta que o benchmark faz.
- **3h** (registrada; não implementada) — atualizar `rmcp` pra `1.x`
  (ou a versão corrente, se for mais nova) e implementar a parte (b)
  que a etapa 3d deixou de fora (§5): a pergunta de aprovação por
  `elicitation` — agente, comando, caminho e efeito, com as opções
  **uma vez**, **sempre nesta sessão** e **negar** (qualquer outra
  resposta, ou nenhuma dentro de um tempo limite de alguns minutos,
  nega — nunca aplica por silêncio). **Onde encaixa**: depois da
  **Decisão** acima, antes de **3b** — as duas opções que o PR de 3d
  pediu pra avaliar são, na prática, a mesma posição na ordem de hoje
  (`decisão` já vem imediatamente antes de `3b`). Depois da decisão,
  não antes, porque é um salto de major version do `rmcp` (API bem
  diferente da `0.3` fixada hoje) — investimento que só vale a pena
  depois de confirmar que o nas vai seguir pelo caminho de edição via
  MCP, não pivotar pra histórico/supervisão. **Não depende do núcleo
  em segundo plano (3b)**: a pergunta por `elicitation` é direto entre
  o cliente MCP e `nas --mcp`, sem precisar do `nas-term` nem do
  daemon — por isso pode vir antes de 3b (como proposto aqui), depois,
  ou em paralelo; a posição só reflete que `decisão`/`3f` já precisam
  ter acontecido primeiro. Rastreada também na issue #32 (rótulo
  `ideia`), pra não depender só deste texto pra não ser esquecida.
- **3b** — núcleo em segundo plano: `nas --daemon`, o socket por
  projeto em `$XDG_RUNTIME_DIR` (0600), o protocolo mínimo, múltiplos
  clientes, autostart/encerramento por inatividade/recuperação de
  socket órfão (§2), e o remapeamento de cursor entre clientes (§2.1,
  com seu teste de aceite); `nas-term` e `nas-mcp` passam a tentar
  conectar antes de cada um criar seu próprio núcleo em processo —
  é aqui que a ressalva de 3c deixa de valer. **Também aqui** (revisão
  da etapa 3d, §5): a interface aberta de permissões — pedido
  bloqueante com tempo limite, echo area do `nas-term` com "uma
  vez"/"sempre na sessão" — que 3d não entregou porque dependia
  justamente desta conexão `nas-term`↔núcleo não existir ainda.
- **3g** — unificação do despacho de comandos (§1): depois que as
  etapas acima provarem o formato do registro na prática, decidir e
  implementar como `nas-keys` passa a resolver (pelo menos parte das)
  suas ações através de `nas-commands`, em vez de despachar pra
  `machine.rs`/`App::apply_action` direto — detalhada quando chegar
  lá, não nesta ADR.

ACP (§4) fica de fora desta lista de propósito.

## Fora de escopo desta fase

- **ACP** (§4) — ADR própria, depois.
- **Lisp/Steel embutido** — adiado, não cancelado; o registro de
  comandos desta fase é Rust puro, sem macro `defcommand`.
- **Persistir "sempre na sessão" automaticamente** no arquivo de
  configuração (§5) — por ora é só leitura manual do que o usuário
  escreveu de antemão.
- **Fusão inteligente de edições sobrepostas** — §2.1 já define
  ordem estrita (FIFO) e remapeamento de posição (cursor, e o
  `edit` por texto exato se protegendo sozinho contra deslocamento);
  o que fica de fora é qualquer coisa além disso, como mesclar duas
  edições que alteram exatamente o mesmo trecho ao mesmo tempo — a
  segunda, nesse caso raro, ainda só se aplica sobre o resultado da
  primeira, na ordem de chegada, sem tentativa de combinar as duas.
- **Desfazer seletivo por autor** (§2.1) — "desfaça só as edições da
  IA, mantendo as minhas" fica como ideia pra depois; por ora,
  desfazer é `u` sobre a última transação de qualquer autor, ou
  restaurar o checkpoint automático da sessão pra desfazer uma
  sessão inteira.

## Consequências

- `CLAUDE.md` precisa de uma atualização (parte do PR de 3a, não
  desta ADR): `nas-commands` deixa de estar reservada pro "marco 3" —
  nasce agora; o roteiro do README também precisa registrar a
  inversão descrita no Contexto.
- Dois modos novos no mesmo binário (`nas --daemon`, `nas --mcp`),
  nenhum binário extra — `nas-mcp` (revisão da etapa 3c) é biblioteca,
  chamada de dentro de `nas-term`; nenhuma crate nova além de
  `nas-commands` e `nas-mcp`.
- A UI de permissão (§5) reaproveita Prompt/Overlay/echo area que já
  existem (inclusive o acabamento which-key desta própria PR) — risco
  baixo sobre o código de interface já estável.
- Tudo que a ADR 0005 já construiu (checkpoint, diff, restauração,
  blame, poda, formato em disco) permanece como está — a única
  superfície nova é registro de comandos + núcleo em segundo plano +
  MCP + permissões; autoria (§6) é composição do que já existe, não
  mecanismo novo.
- `schemars` e `rmcp` são as duas dependências novas desta fase (além
  do que `nas-mcp`/`nas --daemon` precisarem para o transporte —
  provavelmente `tokio` já workspace-presente para o lado do socket).
- `~/.local/state/nas/history/<hash>/` ganha um arquivo novo
  (`permissions.toml`, §5) — mesma pasta por projeto que a ADR 0005 já
  criava, sem precisar de uma pasta nova pra configuração.
