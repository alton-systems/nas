# Benchmark comparativo — fase 3 (etapa 3f)

Registro do teste comparativo descrito em
`docs/decisions/0006-ia-trabalha-pelo-nas.md` §7. Este documento é
preparado na etapa **3f-prep, PASSO 1** — a bateria completa (trinta
rodadas) ainda **não foi rodada**; o que existe até aqui é a
metodologia fixada de antemão (este documento), as cinco tarefas
(`bench/tarefas/`), os scripts (`bench/rodar.sh`, `bench/verificar.py`,
`bench/resumir`) e um ensaio de validação (§5).

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

**nas vs. o Claude Code no seu modo normal** — com as próprias
ferramentas de ler, editar e buscar, além do shell — não "nas vs. só
shell". A pergunta é se `nas-mcp` (leitura com números de linha,
`edit` por texto exato, checkpoint automático de sessão) melhora sobre
o que o Claude Code já faz por conta própria.

- **Lado "normal"**: `Read`, `Edit`, `Write`, `MultiEdit` do Claude
  Code ligadas; shell liberado só para `cargo` e `git`.
- **Lado "nas"**: `Read`, `Edit`, `Write`, `MultiEdit`, `NotebookEdit`
  do Claude Code **desligadas** (`--disallowedTools`); `nas --mcp`
  registrado como único servidor MCP da sessão
  (`--mcp-config`/`--strict-mcp-config`); shell liberado só para
  `cargo` e `git`, igual ao outro lado.

Mesmo modelo, mesma versão do modelo, nos dois lados — fixado abaixo
(§4).

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

Cada arquivo de tarefa tem o texto exato do pedido (idêntico nos dois
lados), o commit de partida, a verificação objetiva e por que a tarefa
é representativa do trabalho real do projeto.

Cada tarefa roda três vezes em cada lado (seis rodadas por tarefa) —
mesmo texto de prompt nas seis — em ordem alternada (lado A, lado B,
lado A, ... tarefa a tarefa), pra não deixar aquecimento de contexto
ou ordem de execução favorecer um lado só por ter rodado primeiro ou
por último.

## 4. Condições fixadas antes de rodar

- **Modelo**: `claude-sonnet-5`, mesma versão nos dois lados — fixado
  em `BENCH_MODEL` (`bench/rodar.sh`); qualquer mudança de modelo
  precisa ser uma decisão explícita, registrada aqui antes de rodar de
  novo.
- **Commit de partida**: `3bfc9bb` (topo da `main` quando as tarefas
  foram escolhidas) — fixado em `BENCH_COMMIT`.
- **Binário `nas`**: compilado **uma única vez**, a partir do mesmo
  commit de partida (`bench/build-nas.sh`), e reaproveitado como
  servidor MCP fixo em toda rodada do lado "nas" — nunca recompilado
  por rodada. É a versão do nas sendo testada; o worktree de cada
  rodada é só o alvo da edição (que, neste benchmark, é o próprio
  código-fonte do nas — a IA usa o nas pra editar o nas).
- **Shell**: `cargo` e `git` liberados nos dois lados, documentado em
  `bench/rodar.sh` (`SHELL_ALLOW`); nenhuma tarefa precisa de rede.
  Variantes prefixadas por `rtk` também liberadas — um hook local
  deste usuário (`~/.claude/RTK.md`) pode reescrever um comando antes
  da checagem de permissão vê-lo; afeta os dois lados igualmente, não
  é um viés entre eles.
- **Política de permissões do lado nas** (ADR 0006 §5):
  `bench/permissoes-benchmark.toml` — libera exatamente
  `write-file = true` (as cinco tarefas são edição de código; nenhuma
  chama shell nem rede a partir de uma ferramenta do nas), copiada
  pro `permissions.toml` de cada worktree antes da rodada.
- **Coleta automatizada**: `claude -p --output-format stream-json`
  (transcrição completa) — nunca transcrita à mão.
- **"Tokens no total"** (critério de decisão, abaixo) = tokens de
  entrada + tokens de criação de cache + tokens de leitura de cache +
  tokens de saída, **somados** — não só entrada+saída. O custo em
  dólares (`total_cost_usd`, já devolvido por `claude -p`) é métrica
  complementar, ao lado, não parte do critério de tokens.

## 5. Ensaio (validação do script — não entra no resultado)

Rodado em 2026-11-25: tarefa 01, uma vez em cada lado, modelo
`claude-sonnet-5`, commit de partida `3bfc9bb`. **Não entra no
resultado** (§7) — serve só pra validar o script, a coleta e a
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

- **Taxa de sucesso** do lado nas igual ou maior que a do lado sem
  nas (contando as três rodadas de cada tarefa).
- **No máximo 25% a mais de tokens no total** (§4) do lado nas,
  somadas as três rodadas de cada tarefa, comparado ao lado sem nas.

Passou nos dois: o nas segue como editor, a fase 3 continua pelas
etapas seguintes (ADR 0006 §8). Falhou em qualquer um dos dois: pivô —
o nas deixa de perseguir ser um editor de propósito geral competitivo
em eficiência de edição, e passa a focar em histórico e supervisão
sobre o que a IA já faz direto, em vez de MCP de edição.

## 7. Resultado

*(preenchido depois da bateria completa — trinta rodadas, cinco
tarefas × dois lados × três rodadas — com `bench/resumir`. Vazio até
lá.)*

## 8. Integridade e rastreabilidade

*(quadro separado — o lado sem nas não tem o recurso equivalente,
então não é uma comparação simétrica como a de tokens/custo/tempo;
é um registro só do lado nas, preenchido depois da bateria completa,
documentando `Ctrl+t e`/`Ctrl+t r` no checkpoint automático de sessão
medido em teclas/comandos, ao lado do que o Claude Code normal
ofereceria pra fazer o mesmo — `git diff`/`git checkout` manual,
anotado como referência, não como "perda" dele no placar.)*
