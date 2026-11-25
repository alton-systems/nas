# Benchmark comparativo — fase 3 (etapa 3f)

Registro do teste comparativo descrito em
`docs/decisions/0006-ia-trabalha-pelo-nas.md` §7. Este documento é
preparado na etapa **3f-prep** — a bateria completa (tamanho a
decidir depois da bateria reduzida, §6/§8) ainda **não foi rodada**; o
que existe até aqui é a metodologia fixada de antemão (este
documento), as cinco tarefas (`bench/tarefas/`), os scripts
(`bench/rodar.sh`, `bench/verificar.py`, `bench/resumir`), um ensaio
de validação com dois cenários (§5) e a bateria reduzida, exploratória,
com os três cenários atuais (§7).

## 1. Ressalva da etapa 3c (ler antes dos números)

Até a decisão pós-benchmark sobre o núcleo em segundo plano (ADR 0006
§2, etapa 3b), `nas --mcp` **não compartilha buffer** com um
`nas-term` eventualmente aberto no mesmo arquivo — a sincronização é
só por disco (ADR 0006 §3/§8). Isso não afeta este benchmark de forma
prática (nenhuma rodada abre um `nas-term` interativo ao lado), mas
fica registrado porque o próprio ADR pede que qualquer lentidão
observada seja atribuída a essa limitação conhecida da etapa, nunca
lida como um problema da integração MCP em si.

## 2. O que está sendo comparado

**Revisão pós-ensaio: três cenários, não dois.** O ensaio (§5) só
comparou "nas obrigatório" contra "normal" — essa comparação continua
(cenário B × cenário A, abaixo), mas não responde se vale mais a pena
**obrigar** o uso do nas ou só **oferecê-lo** ao lado das ferramentas
que o Claude Code já tem. Cenário C existe pra essa segunda pergunta.

- **A (normal)**: `Read`, `Edit`, `Write`, `MultiEdit` do Claude Code
  ligadas; shell liberado só para `cargo` e `git`. Nenhum servidor MCP
  do nas.
- **B (nas obrigatório)**: `Read`, `Edit`, `Write`, `MultiEdit`,
  `NotebookEdit` do Claude Code **desligadas** (`--disallowedTools`);
  `nas --mcp` registrado como único servidor MCP da sessão
  (`--mcp-config`/`--strict-mcp-config`); shell liberado só para
  `cargo` e `git`, igual aos outros dois cenários. A única forma de
  tocar um arquivo é pelas ferramentas do `nas-mcp`.
- **C (híbrido)**: nada desligado — `Read`/`Edit`/`Write`/`MultiEdit`
  do Claude Code ligadas **e** as ferramentas `mcp__nas__*` também
  disponíveis ao mesmo tempo, via MCP; o modelo escolhe livremente
  qual usar pra cada parte da tarefa. Mesmo shell (`cargo`/`git`) dos
  outros dois.

A pergunta de B é "vale a pena **forçar** o nas-mcp?"; a de C é "vale
a pena **oferecer** o nas-mcp, deixando a escolha pro modelo?" — duas
perguntas diferentes, com critérios de decisão diferentes (§6).

Mesmo modelo, mesma versão do modelo, nos três cenários — fixado
abaixo (§4).

## 3. Tarefas

Cinco tarefas reais do backlog do nas, tiradas do estado do repositório
em `3bfc9bb` (`main`), descritas em `bench/tarefas/`:

| # | Categoria | Arquivo |
|---|---|---|
| 01 | corrigir um bug concreto já conhecido (issue #34) | `bench/tarefas/01-bug-checkpoint-sessao.md` |
| 02 | adicionar um teste faltando pra uma função existente | `bench/tarefas/02-teste-parse-atx-heading.md` |
| 03 | renomear algo de forma consistente numa crate | `bench/tarefas/03-renomear-from-to-line.md` |
| 04 | extrair uma pequena duplicação | `bench/tarefas/04-extrair-tracked-files.md` |
| 05 | explicar o que uma função faz (critério escrito antes de rodar) | `bench/tarefas/05-explicar-line-number-for-offset.md` |

Cada arquivo de tarefa tem o texto exato do pedido (idêntico nos três
cenários), o commit de partida, a verificação objetiva e por que a
tarefa é representativa do trabalho real do projeto.

**Bateria completa** (quando os cenários da bateria reduzida, §7,
mostrarem que vale a pena): cada tarefa roda três vezes em cada
cenário que seguir adiante — em ordem alternada entre cenários, tarefa
a tarefa (A, B, C, A, B, C, ... ou só os que sobrarem da decisão),
pra não deixar aquecimento de contexto ou ordem de execução favorecer
um cenário só por ter rodado primeiro ou por último. O número de
rodadas da bateria completa depende de quantos cenários restarem
depois da reduzida — não é fixado aqui de antemão.

## 4. Condições fixadas antes de rodar

- **Modelo**: `claude-sonnet-5`, mesma versão nos três cenários —
  fixado em `BENCH_MODEL` (`bench/rodar.sh`); qualquer mudança de
  modelo precisa ser uma decisão explícita, registrada aqui antes de
  rodar de novo.
- **Commit de partida**: `3bfc9bb` (topo da `main` quando as tarefas
  foram escolhidas) — fixado em `BENCH_COMMIT`.
- **Binário `nas`**: compilado **uma única vez**, a partir do mesmo
  commit de partida (`bench/build-nas.sh`), e reaproveitado como
  servidor MCP fixo em toda rodada dos cenários B/C — nunca
  recompilado por rodada. É a versão do nas sendo testada; o worktree
  de cada rodada é só o alvo da edição (que, neste benchmark, é o
  próprio código-fonte do nas — a IA usa o nas pra editar o nas).
- **Shell**: `cargo` e `git` liberados nos três cenários, documentado
  em `bench/rodar.sh` (`SHELL_ALLOW`); nenhuma tarefa precisa de rede.
  Variantes prefixadas por `rtk` também liberadas — um hook local
  deste usuário (`~/.claude/RTK.md`) pode reescrever um comando antes
  da checagem de permissão vê-lo; afeta os três cenários igualmente,
  não é um viés entre eles.
- **Política de permissões dos cenários B/C** (ADR 0006 §5):
  `bench/permissoes-benchmark.toml` — libera exatamente
  `write-file = true` (as cinco tarefas são edição de código; nenhuma
  chama shell nem rede a partir de uma ferramenta do nas), copiada
  pro `permissions.toml` de cada worktree antes da rodada.
- **`ENABLE_TOOL_SEARCH=false` nos cenários B/C** (revisão pós-
  benchmark, item 1c): sem isto, o modelo precisa descobrir o esquema
  das ferramentas `mcp__nas__*` chamando `ToolSearch` antes de usá-las
  — um custo de turnos extras só do lado que usa MCP, medido no
  ensaio. Com a variável desativando a descoberta sob demanda, o
  esquema é carregado de uma vez no início da sessão (documentado em
  <https://code.claude.com/docs/en/mcp.md>, "Disabling Tool Search") —
  só muda **quando** o esquema chega, nunca **quais** ferramentas
  estão liberadas (`--allowedTools` continua decidindo isso). Não
  aplicado no cenário A, que não tem nenhum servidor MCP configurado.
- **Aviso de execução não assistida, igual nos três cenários**
  (`--append-system-prompt` em `bench/rodar.sh`): *"Execução
  automatizada de benchmark: não há ninguém para aprovar. Conclua a
  tarefa por completo, sem pedir confirmação. Não faça commit nem
  push."* Sem isso, o ensaio mostrou um risco real de viés: o lado nas
  leu o próprio `CLAUDE.md` do worktree (que é, afinal, uma cópia de
  trabalho do repositório do nas) e parou antes de terminar, "esperando
  revisão" — um comportamento que mediria estilo de parada entre os
  lados, não a tarefa em si, se um lado parasse mais cedo que o outro
  por esse motivo. O aviso não dispensa a verificação objetiva de cada
  tarefa (`bench/verificar.py`) nem muda a política de permissões
  acima — só deixa explícito que, nesta execução, parar pra pedir
  aprovação não é uma opção válida.
- **Coleta automatizada**: `claude -p --output-format stream-json`
  (transcrição completa) — nunca transcrita à mão.
- **"Tokens no total"** (critério de decisão, abaixo) = tokens de
  entrada + tokens de criação de cache + tokens de leitura de cache +
  tokens de saída, **somados** — não só entrada+saída. O custo em
  dólares (`total_cost_usd`, já devolvido por `claude -p`) é métrica
  complementar, ao lado, não parte do critério de tokens.

## 5. Ensaio (validação do script — não entra em nenhum resultado)

**Nota de terminologia**: este ensaio rodou antes da revisão pós-
benchmark que introduziu o cenário C (§2) — "lado normal"/"lado nas"
abaixo correspondem aos cenários **A** e **B** de hoje, não a um
terceiro rótulo.

Rodado em 2026-11-25: tarefa 01, uma vez em cada lado, modelo
`claude-sonnet-5`, commit de partida `3bfc9bb`. **Não entra em nenhum
resultado** (§7/§8) — serve só pra validar o script, a coleta e a
verificação antes de rodar a bateria completa (trinta rodadas).

| tarefa | lado | tok. entrada | tok. criação cache | tok. leitura cache | tok. saída | **tok. total** | custo (US$) | chamadas | tempo (s) | sucesso |
|---|---|---|---|---|---|---|---|---|---|---|
| 01 | normal | 84 | 66 159 | 2 558 866 | 16 680 | **2 641 789** | 0,9448 | 41 | 208 | ✅ |
| 01 | nas | 120 | 105 573 | 4 344 280 | 25 630 | **4 475 603** | 1,5491 | 61 | 323 | ✅ |

Os dois lados corrigiram a issue #34 corretamente (`cargo test
--workspace` passou nos dois, e os dois acrescentaram um teste de
regressão que de fato reproduz o bug antigo e passa com a correção —
confirmado lendo a resposta final de cada lado, não só os números).
Nesta única amostra (n=1 por lado — não é o resultado, só o ensaio),
o lado nas usou ~69% mais tokens no total e levou ~55% mais tempo de
parede que o lado normal; dentro do esperado pela ressalva da etapa
3c (§1): sem buffer compartilhado, o lado nas paga o custo de
`open_file`/`read`/`sync_with_disk` como chamadas de ferramenta
separadas onde o lado normal só usa `Read`/`Edit` nativos — é
exatamente a pergunta que a bateria completa (três rodadas por lado)
precisa responder com uma amostra maior, não com n=1.

**O que o ensaio corrigiu antes da bateria real**: a verificação
objetiva automatizada da tarefa 01 (`bench/verificar.py`) só procurava
o teste novo em `registry.rs`; o lado normal pôs o teste em
`commands.rs` (ao lado dos outros testes de checkpoint de sessão) — a
implementação estava certa, só a verificação apontou falso-negativo.
Corrigido para ler o crate inteiro antes de qualquer rodada real.

**Efeito colateral a limpar depois da bateria completa**: cada rodada
do lado nas cria um diretório em
`~/.local/state/nas/history/<hash>/` (um por caminho de worktree,
já que o hash do projeto depende do caminho) — nada grave (cada um
tem poucos KB), mas acumula um por rodada; `rm -rf
~/.local/state/nas/history/<hash>/` depois de cada rodada, ou uma
limpeza no fim da bateria, evita que isso fique pra trás sem querer.

**Quanto o ensaio consumiu**, pra estimar o custo da bateria completa
(trinta rodadas — quinze por lado, cinco tarefas × três rodadas):
custo total do ensaio = **US$ 2,49** (0,9448 + 1,5491), tempo de
parede total = **531 s** (~9 min, as duas rodadas em paralelo
levaram só o tempo da mais longa, 323 s, já que correram ao mesmo
tempo — mas o custo em dólar soma os dois). Se as outras quatro
tarefas tiverem um custo por rodada parecido ao da tarefa 01 (a mais
simples das cinco, pelo volume de código envolvido — tarefas maiores
como a 02/04 provavelmente custam mais, não menos), a bateria
completa de trinta rodadas fica em torno de **US$ 35–50**, rodando em
paralelo por lado (quinze rodadas de cada lado, em sequência
alternada dentro do mesmo lado — ADR 0006 §7 pede ordem alternada
entre lados, tarefa a tarefa, não paralelismo despreocupado) — uma
estimativa grosseira (um fator de 15 sobre uma amostra de 1), não uma
previsão fina.

## 6. Critério de decisão (fixado antes de rodar)

Dois critérios, um por comparação (§2) — **fixados antes de qualquer
rodada real, pra não racionalizar o resultado depois de ver os
números**. Os dois usam "tokens no total" na mesma definição do §4
(entrada + criação de cache + leitura de cache + saída, somados) e
"taxa de sucesso" na mesma definição do §3/`bench/verificar.py`.

- **B × A** (vale a pena **obrigar** o nas-mcp?): taxa de sucesso de B
  igual ou maior que a de A, **e** no máximo **25%** a mais de tokens
  no total em B, somadas as rodadas de cada tarefa, comparado a A.
  Este é o critério original, já usado para avaliar o ensaio (§5).
- **C × A** (vale a pena **oferecer** o nas-mcp, sem obrigar?): taxa
  de sucesso de C igual ou maior que a de A, **e** no máximo **10%** a
  mais de tokens no total em C, comparado a A — uma barra mais
  apertada que a de B×A, porque C tem a opção de nunca usar o nas-mcp
  quando ele não ajudar; um C que custe quase tanto quanto B sem
  precisar usar o nas-mcp (e o quadro de uso em §7 mostrar isso) seria
  sinal de que oferecer a ferramenta, por si só, não está valendo a
  diferença de custo.

**A bateria reduzida (§7) é exploratória — não decide nada por si
só.** Ela roda só uma vez por cenário, por tarefa (quinze rodadas no
total), pra ver a direção dos números e se faz sentido seguir com os
três cenários, só dois, ou nenhum, antes de pagar o custo da bateria
completa (três rodadas por cenário que seguir adiante). A decisão de
verdade — se o nas segue como editor (ADR 0006 §8), se vira "nas
opcional" (nem B nem C valem o custo de obrigar, mas C ainda ajuda
quando escolhido), ou se pivota pra histórico/supervisão (nenhum dos
dois criterios passa em nenhum cenário) — usa a bateria completa, não
a reduzida.

## 7. Bateria reduzida (exploratória)

Rodada em 2026-11-25, depois das correções 1a/1b/1c (commits
`4ede370`/`3dc54c6`) — binário `nas` compilado do HEAD da branch com
as correções já aplicadas, não do commit de partida das tarefas. Cinco
tarefas × três cenários × uma rodada = quinze rodadas. A ordem alternada
planejada em §3 não foi seguida à risca na execução (rodadas disparadas
em paralelo, não em sequência — processos independentes, sem contexto
compartilhado entre si, então a alternância não se aplicava do mesmo
jeito que numa bateria manual sequencial); duas rodadas da primeira
tentativa (o orquestrador `bateria-reduzida.sh` original) caíram num
limite de sessão da conta no meio do caminho (`429`, "You've hit your
session limit") e tiveram que ser refeitas uma a uma depois que o
limite resetou — sem isso, `wait -n` sob `set -e` matava o script
inteiro sem limpar os processos filhos, que ficaram órfãos; corrigido
não no script (que já não roda mais sem supervisão), mas relançando
cada rodada que faltou individualmente. Três rodadas da tarefa 05
(todas de A/B/C) perderam o `verification.json`/`meta.json` nesse
mesmo incidente — como a tarefa 05 é só explicação (confirmado,
lendo as três transcrições: zero chamadas de edição em qualquer uma),
o `cargo_test_ok`/tempo de parede foram reconstruídos à mão a partir
do `result.json` (que não se perdeu) em vez de gastar uma rodada nova.

| tarefa | cenário | tok. entrada | tok. criação cache | tok. leitura cache | tok. saída | **tok. total** | custo (US$) | chamadas | tempo (s) | sucesso | retrabalho | edições via shell |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 01 | A | 76 | 87 786 | 2 270 866 | 12 062 | **2 370 790** | 0,9275 | 37 | 236 | ✅ | 4 | 0 |
| 01 | B | 54 | 93 222 | 1 886 234 | 12 221 | **1 991 731** | 0,8739 | 26 | 215 | ✅ | 1 | 0 |
| 01 | C | 70 | 106 520 | 2 728 971 | 15 761 | **2 851 322** | 1,1311 | 35 | 262 | ✅ | 7 | 0 |
| 02 | A | 8 | 19 290 | 118 454 | 1 772 | **139 524** | 0,1201 | 3 | 26 | ✅ | 0 | 0 |
| 02 | B | 10 | 52 226 | 197 672 | 1 744 | **251 652** | 0,2674 | 4 | 28 | ✅ | 0 | 0 |
| 02 | C | 10 | 54 174 | 206 799 | 1 941 | **262 924** | 0,2790 | 4 | 29 | ✅ | 0 | 0 |
| 03 | A | 22 | 22 149 | 377 096 | 3 167 | **402 434** | 0,1971 | 10 | 70 | ✅ | 1 | 0 |
| 03 | B | 20 | 22 397 | 484 481 | 3 122 | **510 020** | 0,2191 | 14 | 80 | ✅ | 2 | 0 |
| 03 | C | 24 | 21 287 | 616 325 | 3 258 | **640 894** | 0,2425 | 11 | 81 | ✅ | 2 | 0 |
| 04 | A | 42 | 33 780 | 832 801 | 5 162 | **871 785** | 0,3547 | 21 | 156 | ✅ | 1 | 0 |
| 04 | B | 24 | 26 703 | 614 673 | 3 053 | **644 453** | 0,2617 | 13 | 123 | ✅ | 0 | 0 |
| 04 | C | 50 | 38 324 | 1 514 422 | 5 443 | **1 558 239** | 0,5121 | 24 | 157 | ✅ | 6 | 2\* |
| 05 | A | 8 | 16 076 | 116 758 | 1 565 | **134 407** | 0,1046 | 3 | 21 | — | 0 | 0 |
| 05 | B | 6 | 14 101 | 127 740 | 2 059 | **143 906** | 0,1039 | 2 | 29 | — | 0 | 0 |
| 05 | C | 8 | 15 241 | 187 448 | 2 240 | **204 937** | 0,1222 | 3 | 27 | — | 0 | 0 |

\* falso-positivo da heurística de "edição pelo shell" (§4): os dois
comandos eram `cargo test --workspace > /tmp/test_out.log` — um
redirecionamento pra inspecionar a saída, não uma edição de arquivo do
projeto. A heurística (`bench/resumir`) marca qualquer `>`/`>>` num
comando `Bash`, sem saber se o destino é um arquivo do projeto ou um
log temporário — ponto a refinar antes da bateria completa, não um
resultado real de "edição via shell contornando o nas".

**Custo total das quinze rodadas: US$ 5,7168.** Tempo de parede total
somado (sem contar o paralelismo e os reinícios pelo limite de sessão)
não é uma métrica útil aqui — o que importa pro §6 é tokens e sucesso.

### Uso das ferramentas do nas no cenário C

**Nenhuma.** Nas cinco tarefas, zero chamadas a qualquer ferramenta
`mcp__nas__*` — confirmado tanto por `bench/resumir --reduzida --json`
quanto por inspeção direta das transcrições. Com as duas ferramentas
disponíveis (Claude Code nativo e nas-mcp), o modelo não escolheu o
nas-mcp nenhuma vez nesta amostra.

### Contra os critérios do §6 (agregado das cinco tarefas, n=1 por célula — exploratório)

| comparação | tokens totais (soma das 5 tarefas) | diferença | critério | resultado nesta amostra |
|---|---|---|---|---|
| B × A | B = 3 541 762 / A = 3 918 940 | **−9,6%** (B mais barato) | ≤ 25% a mais | dentro do critério, com folga |
| C × A | C = 5 518 316 / A = 3 918 940 | **+40,8%** | ≤ 10% a mais | **fora do critério**, por uma margem grande |

Taxa de sucesso: ✅ nas quatro tarefas verificáveis (01–04), nos três
cenários — sem diferença de sucesso entre eles nesta amostra. Tarefa
05 (explicar) não entra nesta soma — precisa da nota manual de 5
pontos (critério do próprio `bench/tarefas/05-...md`), ainda não feita
pra estas três respostas.

**Leitura, com a ressalva de que isto é exploratório (n=1 por
tarefa/cenário, não a bateria completa):**

- **As correções 1a/1b/1c (item 1, revisão pós-ensaio) funcionaram**:
  o ensaio original (§5) media o cenário B ~70% mais caro que A; nesta
  bateria, com as ferramentas corrigidas, B ficou **mais barato** que
  A. O `save` por padrão (menos chamadas de `save_file`) e
  `ENABLE_TOOL_SEARCH=false` (menos `ToolSearch`) parecem ter resolvido
  justamente o que o ensaio tinha apontado.
- **O cenário C saiu caro sem nunca ser usado.** C custou ~41% mais
  que A somando as cinco tarefas, numa amostra em que o nas-mcp nunca
  foi chamado nenhuma vez — ou seja, o custo extra de C, nesta amostra,
  não é o custo de *usar* o nas-mcp (que seria zero, já que não foi
  usado): é o custo de só **tê-lo disponível** (esquema de doze
  ferramentas a mais carregado na sessão, mesmo com
  `ENABLE_TOOL_SEARCH=false` evitando o `ToolSearch`, ainda ocupa
  espaço — e possivelmente mais raciocínio sobre qual ferramenta
  escolher). Isso é uma evidência, ainda que de amostra pequena, contra
  a hipótese de que "oferecer sem obrigar" seria uma forma barata de
  deixar a IA escolher: aqui ela nunca escolheu, e ainda assim pagou
  mais.
- **Isto não decide nada ainda** (§6) — é a bateria completa, com três
  rodadas por tarefa e tarefas maiores que a 01, que decide de
  verdade. Mas a direção desta amostra sugere que vale a pena levar
  **B** pra bateria completa (o critério já passa, com folga, mesmo
  nesta amostra pequena) e repensar — ou simplesmente não repetir — o
  **C** como está, já que não houve nenhum uso de nas-mcp pra justificar
  o custo extra observado.

## 8. Resultado (bateria completa)

*(preenchido depois da bateria completa, cujo tamanho depende da
decisão tomada com a bateria reduzida (§6/§7) — com `bench/resumir`.
Vazio até lá.)*

## 9. Integridade e rastreabilidade

*(quadro separado — o lado sem nas não tem o recurso equivalente,
então não é uma comparação simétrica como a de tokens/custo/tempo;
é um registro só do lado nas, preenchido depois da bateria completa,
documentando `Ctrl+t e`/`Ctrl+t r` no checkpoint automático de sessão
medido em teclas/comandos, ao lado do que o Claude Code normal
ofereceria pra fazer o mesmo — `git diff`/`git checkout` manual,
anotado como referência, não como "perda" dele no placar.)*
