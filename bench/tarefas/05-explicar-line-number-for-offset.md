# Tarefa 5 — explicar o que uma função faz

## Categoria

Explicar o que uma função faz. Como não há `cargo test` que avalie uma
explicação em texto, o critério de nota vem **escrito antes de rodar**
(abaixo), pra não racionalizar a avaliação depois de ler as duas
respostas.

## Commit de partida

`3bfc9bb` (`main`).

## Texto exato do pedido

> Explique, em português, o que a função `line_number_for_offset`
> (`crates/nas-commands/src/commands.rs`, perto da linha 144) faz.
> Responda especificamente a estas cinco perguntas:
>
> 1. Em que unidade `offset` é contado — bytes ou caracteres Unicode
>    (`char`)?
> 2. O número de linha devolvido é 0-indexado ou 1-indexado?
> 3. O que acontece quando `offset` cai exatamente sobre um caractere
>    de nova linha (`'\n'`)? Esse caractere conta para a linha
>    anterior ou para a próxima?
> 4. O que a função devolve quando `offset` é igual ou maior que o
>    número total de caracteres de `text` (um offset "passado do
>    fim")? Dê o valor exato para um exemplo concreto, não só "um
>    número maior".
> 5. Qual é a complexidade de tempo da função em relação ao tamanho de
>    `offset` e de `text`? Isso importa para como `edit()` a usa (ela é
>    chamada duas vezes por chamada de `edit`, sobre o texto já
>    atualizado)?
>
> Não é necessário alterar nenhum código — só responder, por escrito,
> às cinco perguntas.

(Mesmo texto nos dois lados.)

## Critério de nota (escrito antes de rodar)

Cinco pontos possíveis, um por pergunta — a resposta de cada lado é
lida às cegas (sem saber qual foi gerada com nas e qual sem) e marcada
certa ou errada contra o que segue, tirado do código de
`line_number_for_offset` no commit de partida:

1. **Certo** = afirma que `offset` é contado em caracteres Unicode
   (`.chars()`), não em bytes. (A função itera `text.chars().enumerate()`
   — um texto com qualquer caractere multi-byte revelaria a diferença.)
2. **Certo** = afirma que o resultado é 1-indexado (a primeira linha é
   `1`, a variável `line` começa em `1`).
3. **Certo** = afirma que o `'\n'` em si pertence à linha que ele
   termina, não à próxima — porque o `break` do laço acontece **antes**
   de incrementar `line` para aquele caractere. Concretamente, para
   `text = "ab\ncd\n"`, `line_number_for_offset(text, 2)` (o offset do
   próprio `'\n'`) devolve `1`, não `2`.
4. **Certo** = afirma que, para um offset igual ou maior que
   `text.chars().count()`, a função continua contando **todo** `'\n'`
   do texto inteiro, podendo devolver um número **maior** que a
   contagem real de linhas de `text.lines()` quando o texto termina em
   quebra de linha. Concretamente, para `text = "ab\ncd\n"` (que
   `.lines()` conta como 2 linhas), `line_number_for_offset(text, 6)`
   devolve `3` — uma linha além do total real. Crédito parcial (meio
   ponto) para quem só diz "devolve um número maior que o esperado"
   sem a magnitude exata.
5. **Certo** = identifica que é uma busca linear, `O(offset)` (ou
   equivalentemente `O(n)` no tamanho de `text` no pior caso) — não
   `O(1)` — e observa que, como `edit()` chama a função duas vezes por
   chamada (não dentro de um laço sobre todas as edições), o custo
   total continua linear por chamada, não quadrático; é um ponto de
   atenção de desempenho, não um bug.

**Sucesso** = pelo menos 4 dos 5 pontos corretos (aceitando os meios
pontos de (4) como 0,5). Menos que isso é insucesso para efeito da
métrica "sucesso" do benchmark — mesmo que o texto geral pareça bem
escrito.

## Por que é representativa

"Explicar o que uma função faz" é um pedido comum e genuinamente útil
(onboarding, revisão de código, documentação) que não tem uma
verificação automática — por isso o próprio ADR 0006 §7 pede que,
pra essa categoria, o critério venha escrito antes de rodar.
`line_number_for_offset` foi escolhida por ter comportamento não óbvio
o bastante (unidade de contagem, borda do `'\n'`, offset fora dos
limites) pra separar uma explicação que só leu a assinatura de uma que
de fato rastreou a lógica — sem ser tão longa ou acoplada a outras
partes do código que a resposta correta dependesse de abrir vários
arquivos.
