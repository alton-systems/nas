# Tarefa 3 — renomear algo de forma consistente numa crate

## Categoria

Renomear algo de forma consistente numa crate.

## Commit de partida

`3bfc9bb` (`main`).

## Texto exato do pedido

> No crate `nas-commands`, em `crates/nas-commands/src/commands.rs`, o
> conceito de "intervalo de linhas" aparece com dois nomes diferentes
> dentro do mesmo arquivo: os argumentos públicos de `ReadArgs` (o que
> a IA vê via MCP) usam `start_line`/`end_line`, mas as funções
> internas `numbered_lines` e `char_range_for_lines` (perto das linhas
> 132 e 175) recebem parâmetros chamados `from_line`/`to_line`, e o
> ponto de chamada em `edit()` desestrutura `line_range` como
> `Some([from_line, to_line])` (perto da linha 404).
>
> Renomeie de forma consistente: troque `from_line`/`to_line` por
> `start_line`/`end_line` em toda a assinatura e corpo de
> `numbered_lines` e `char_range_for_lines`, no ponto de chamada
> dentro de `edit()`, e nos comentários de documentação dessas duas
> funções que mencionem `from_line`/`to_line` — sem mudar nenhum
> comportamento, só os nomes, pra bater com a convenção que `ReadArgs`
> já usa.
>
> Não toque nas variáveis locais `edit_start_line`/`edit_end_line` de
> dentro de `edit()` (já usam essa convenção) nem no nome do campo
> `line_range` em si (o argumento JSON que a IA envia não muda, só os
> parâmetros internos que o recebem).

(Mesmo texto nos dois lados.)

## Verificação objetiva

1. `cargo test --workspace` passa, sem nenhuma mudança de
   comportamento observável (é um rename puro).
2. Depois da mudança, o comando

   ```sh
   grep -nE '\bfrom_line\b|\bto_line\b' crates/nas-commands/src/commands.rs
   ```

   não deve dar nenhum resultado (hoje dá vários: a assinatura e o
   corpo de `numbered_lines`/`char_range_for_lines`, mais o ponto de
   chamada em `edit()` e os comentários que os descrevem).

## Por que é representativa

Uma inconsistência de nomes pequena, real, dentro de um único arquivo
que já estava sendo trabalhado nesta mesma etapa (3f-prep, que
acrescentou `start_line`/`end_line` a `ReadArgs` sem atualizar as
funções internas que já existiam antes com outro nome) — exatamente o
tipo de limpeza que fica pendente depois de uma etapa focada em
funcionalidade nova, sem nenhuma mudança de comportamento, só
legibilidade. Não exige entender nenhuma regra de negócio nova, só
seguir consistentemente um nome já estabelecido em outra parte do
mesmo arquivo.
