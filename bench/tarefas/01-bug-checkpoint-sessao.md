# Tarefa 1 — corrigir um bug concreto já conhecido (issue #34)

## Categoria

Corrigir um bug concreto já conhecido.

## Commit de partida

`3bfc9bb` (`main`, topo no momento em que as tarefas foram escolhidas).

## Texto exato do pedido

> No crate `nas-commands`, em `crates/nas-commands/src/registry.rs`, a
> função `session_checkpoint_exists` decide se uma sessão de IA já tem
> o checkpoint automático de sessão (ADR 0006 §6) procurando nos
> checkpoints salvos em disco (`nas_history::list_checkpoints`) a cada
> chamada de `prepare_session_checkpoint`. Isso é a issue #34 do
> repositório: se a poda de checkpoints (`nas_history::prune`) apagar
> o checkpoint automático de uma sessão longa, a próxima chamada
> efetiva da mesma sessão não encontra mais o checkpoint, cria um
> segundo checkpoint automático no meio da sessão, e restaurar esse
> segundo checkpoint não desfaz a sessão inteira (só a parte depois
> dele).
>
> Corrija isso: em vez de procurar nos checkpoints salvos, guarde em
> memória, por `Workspace` (o processo vive o tempo da sessão), quais
> sessões (`agent`, `session`, de `Author::Ai`) já receberam seu
> checkpoint automático — um `HashSet` ou equivalente — e consulte
> essa marca em memória em vez de `nas_history::list_checkpoints`.
> Depois da correção, nem `prepare_session_checkpoint` nem
> `session_checkpoint_exists` devem mais chamar
> `nas_history::list_checkpoints`.
>
> Adicione um teste que prove o comportamento correto: criar o
> checkpoint automático de uma sessão, apagar esse checkpoint do disco
> diretamente (simulando a poda), fazer uma segunda chamada efetiva da
> mesma sessão, e confirmar que nenhum checkpoint automático novo foi
> criado — porque a marca em memória já sabe que a sessão já teve o
> seu, mesmo sem o checkpoint antigo mais existir no disco.

(Mesmo texto nos dois lados — "nas" e "normal" — na íntegra, sem
adaptação.)

## Verificação objetiva

1. `cargo test --workspace` passa.
2. Dentro de `prepare_session_checkpoint`/`session_checkpoint_exists`
   (`crates/nas-commands/src/registry.rs`), não há mais nenhuma
   chamada a `nas_history::list_checkpoints` — checável com:

   ```sh
   ! awk '/fn prepare_session_checkpoint/,/^}/' crates/nas-commands/src/registry.rs \
       crates/nas-commands/src/registry.rs | grep -q list_checkpoints
   ```

   (a forma exata do comando pode precisar de ajuste depois que o
   código mudar de forma; o espírito é "nenhuma das duas funções chama
   `list_checkpoints`").
3. Existe pelo menos um teste novo (não existente no commit de
   partida) que cria o checkpoint de sessão, remove o diretório desse
   checkpoint do disco, dispara uma segunda chamada efetiva da mesma
   sessão, e falha se um segundo checkpoint automático for criado.

Sucesso = os três itens acima, não só o primeiro — passar os testes
sem de fato remover a dependência de `list_checkpoints` (ex.: só
ignorando o resultado de uma poda simulada de outra forma) não conta
como corrigido.

## Por que é representativa

É uma issue real já registrada no rastreador (#34), não inventada para
o benchmark — nasceu de uma leitura cuidadosa do próprio código durante
a etapa 3e, não de uma vulnerabilidade sintética. Mexe em exatamente
um crate (`nas-commands`), em duas funções pequenas e já comentadas
explicando a regra de negócio (ADR 0006 §6), com uma correção
mecânica (ler um `HashSet` em memória em vez de um arquivo em disco) —
do tamanho de uma tarefa real de backlog, nem grande nem ambígua
demais para caber nas três rodadas por lado.
