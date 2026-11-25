# Tarefa 2 — adicionar um teste faltando pra uma função existente

## Categoria

Adicionar um teste faltando pra uma função existente.

## Commit de partida

`3bfc9bb` (`main`).

## Texto exato do pedido

> No crate `nas-syntax`, em `crates/nas-syntax/src/markdown.rs`, a
> função privada `parse_atx_heading(line: &str) -> Option<(usize,
> String)>` (perto da linha 47) hoje só é exercitada indiretamente,
> através dos testes de `list_symbols` no mesmo arquivo. Adicione
> testes diretos — um ou mais `#[test]` novos que chamem
> `parse_atx_heading` diretamente, não `list_symbols` — cobrindo pelo
> menos estes casos:
>
> 1. cada nível de 1 a 6 cerquilhas (`#` a `######`) é aceito, com o
>    nível certo devolvido;
> 2. sete ou mais cerquilhas seguidas (`####### texto`) não é título —
>    devolve `None`;
> 3. uma cerquilha sem espaço nem tabulação depois (`#foo`) não é
>    título — devolve `None`;
> 4. uma tabulação depois da cerquilha (`#\ttítulo`) é aceita como
>    separador, do mesmo jeito que um espaço;
> 5. cerquilhas de fechamento opcionais são removidas do texto — `##
>    título ##` deve devolver `Some((2, "título".to_string()))`, sem
>    as cerquilhas de fechamento nem os espaços ao redor delas;
> 6. um título vazio — só a cerquilha e nada mais depois do espaço
>    (`#` seguido só de espaço, ou de nada) — devolve o nível certo
>    com texto vazio (`Some((1, String::new()))`), não `None`.
>
> Não mude o comportamento de `parse_atx_heading` — só adicione os
> testes que faltam, no mesmo estilo dos testes que já existem nesse
> arquivo (nomes de teste descritivos em português, como os que já
> estão lá).

(Mesmo texto nos dois lados.)

## Verificação objetiva

1. `cargo test --workspace` passa.
2. Depois da mudança, existe pelo menos uma chamada direta a
   `parse_atx_heading(...)` dentro de `mod tests` de
   `crates/nas-syntax/src/markdown.rs` (hoje a única chamada é de
   dentro de `list_symbols`, fora do módulo de testes) — checável com:

   ```sh
   awk '/mod tests/,0' crates/nas-syntax/src/markdown.rs | grep -c "parse_atx_heading("
   ```

   deve devolver 1 ou mais (hoje devolve 0).
3. Os seis casos listados no pedido devem estar, de fato, cobertos —
   revisão manual dos novos testes contra a lista acima (não há um
   jeito totalmente mecânico de checar isso; é parte da verificação
   humana desta tarefa).

## Por que é representativa

"Escrever o teste que falta" é trabalho constante neste repositório —
praticamente toda etapa da fase 3 (3a–3f-prep) termina com uma rodada
de testes novos para o que a etapa anterior deixou sem cobertura
direta. `parse_atx_heading` é uma função pura, pequena, sem
dependência de disco nem de `Workspace`, com vários ramos (nível,
separador, cerquilha de fechamento, texto vazio) que já têm
comportamento definido no código — exatamente o perfil de função que
merece teste direto em vez de só indireto.
