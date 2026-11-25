# Tarefa 4 — extrair uma pequena duplicação

## Categoria

Extrair uma pequena duplicação.

## Commit de partida

`3bfc9bb` (`main`).

## Texto exato do pedido

> No crate `nas-commands`, a mesma lógica aparece duplicada em dois
> lugares: na função `checkpoint_create`
> (`crates/nas-commands/src/commands.rs`, perto da linha 715) e na
> função `prepare_session_checkpoint`
> (`crates/nas-commands/src/registry.rs`, perto da linha 292). Nos dois
> lugares, o código chama `nas_history::list_tracked(&history_dir)` e
> depois, para cada entrada rastreada, busca o conteúdo atual com
> `ctx.workspace.current_content_for(&entry.path)`, descartando (via
> `filter_map`) qualquer entrada cujo conteúdo não esteja disponível,
> pra montar um `Vec<(String, Vec<u8>)>` de (caminho, conteúdo) — o
> formato que `nas_history::create_checkpoint` espera.
>
> Extraia essa lógica para uma função só — no lugar que achar mais
> natural (um método de `Workspace`, em `context.rs`, ou uma função
> livre que receba o `Workspace` e o `history_dir`) — e troque as duas
> duplicações por uma chamada a essa função. Não mude o comportamento:
> os dois lugares devem continuar produzindo exatamente a mesma lista
> de antes, na mesma ordem, com as mesmas entradas descartadas quando o
> conteúdo não está disponível.

(Mesmo texto nos dois lados.)

## Verificação objetiva

1. `cargo test --workspace` passa, sem mudança de comportamento.
2. Depois da mudança, o comando

   ```sh
   grep -rn "current_content_for(&entry.path)" crates/nas-commands/src/
   ```

   deve mostrar exatamente uma ocorrência (hoje mostra duas — uma em
   `commands.rs`, outra em `registry.rs`).

## Por que é representativa

Duplicação exata, byte a byte quase idêntica, entre dois arquivos do
mesmo crate que foram escritos em etapas diferentes (`checkpoint_create`
é da etapa 3a; `prepare_session_checkpoint`, da 3e) sem que quem
escreveu a segunda notasse que a primeira já tinha a mesma lógica —
o cenário mais comum de duplicação num projeto real, não um exemplo
fabricado. A extração é pequena (poucas linhas) e de baixo risco
(função pura o bastante, sem efeito colateral a preservar além do
`Vec` devolvido).
