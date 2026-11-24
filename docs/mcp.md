# MCP no nas

Referência de `nas --mcp` — a decisão está em
`docs/decisions/0006-ia-trabalha-pelo-nas.md` (§3, §5, §8: etapa 3c);
este documento é só como usar o que já existe, não as razões por
trás.

**Limitações conhecidas desta etapa** (leia antes de testar, pra não
confundir com defeito):

- **Sem buffer compartilhado com o `nas-term` (até a etapa 3b).**
  `nas --mcp` mantém seu próprio núcleo dentro do processo, igual ao
  `nas-term` — os dois não compartilham memória. Uma edição da IA só
  chega a um `nas-term` aberto no mesmo arquivo pelo disco, pela
  mesma detecção de mudança externa de sempre (`docs/decisions/0005`
  §1): se o buffer dele estiver limpo, recarrega sozinho; se tiver
  alterações, avisa e espera `Ctrl+r`.
- **Sem pergunta interativa (`elicitation`) ainda.** O `rmcp` fixado
  neste workspace (`0.3`) não suporta `elicitation` — só chegou em
  versões bem posteriores (major `1.0+`). Até o workspace migrar pra
  uma versão que suporte (e o cliente MCP também — o Claude Code
  suporta desde a versão `2.1.76`), o único jeito de liberar
  `WriteFile`/`Shell`/`Network` é de antemão, pelo arquivo de política
  (`nas --permissions`, abaixo) — nunca uma pergunta no meio da
  sessão, nem "sempre nesta sessão" (ADR 0006 §5, revisão da etapa
  3d).
- **Sem checkpoint automático de sessão (até a etapa 3e).** Cada
  sessão de IA ainda não ganha um checkpoint próprio no começo; use
  `checkpoint_create` manualmente se quiser um ponto pra voltar antes
  de deixar a IA trabalhar.

## Como registrar no Claude Code

`nas --mcp` fala stdio — o transporte que o Claude Code espera,
lançando o comando como subprocesso:

```sh
claude mcp add nas -- /caminho/para/nas --mcp
```

O servidor descobre a raiz do projeto do mesmo jeito que o histórico
(`docs/decisions/0005` §6: raiz do Git, ou o diretório onde foi
iniciado) — registre o comando a partir do diretório do projeto, ou
confira que o Claude Code inicia o subprocesso lá.

Confirme com `claude mcp list` — o nas deve aparecer como `connected`
depois da primeira chamada.

## Permissões

`open_file`, `read`, `search`, `list_files`, `edit` e
`checkpoint_create` rodam direto — são `Pure`, `ReadFile`,
`WriteBuffer` ou `WriteHistory` (ADR 0006 §5): nenhum toca o disco do
projeto de verdade, nem sai dele. `create_file`, `checkpoint_restore`
e `save_file` são `WriteFile` — pedem aprovação.

A aprovação, nesta etapa, vem só de um arquivo:

```sh
nas --permissions
```

Mostra o caminho de `permissions.toml` deste projeto (fora do
repositório, em `~/.local/state/nas/history/<hash>/`, nunca dentro
dele — nada no projeto pode se autoaprovar) e a política atual; cria
um modelo comentado, com tudo negado, se ainda não existir. Edite o
arquivo à mão e marque `true` o que quiser liberar:

```toml
write-file = true
shell = false
network = false
```

Sem o arquivo, ou com o efeito marcado `false` (ou ausente), o
comando é recusado com um erro claro dizendo o que falta e como
liberar. Um arquivo inválido (TOML malformado, chave desconhecida,
valor que não é booleano) nega **tudo**, com o motivo na própria
mensagem — nunca aplica a metade que deu certo (mesma ideia de
configuração recuperável da `docs/decisions/0007` §3). Cada pedido —
negado ou aprovado — é registrado em
`~/.local/state/nas/history/<hash>/permissions_log.jsonl`.

Não existe ainda uma pergunta interativa no meio da sessão (ver a
limitação no topo deste documento) — só o arquivo, lido de antemão.

## Como testar à mão

Sem nenhum cliente MCP por perto, dá pra falar o protocolo
diretamente por stdio. `initialize` precisa vir antes de qualquer
outra chamada; depois dele, `notifications/initialized` (sem
resposta) e então `tools/list`/`tools/call`:

```sh
cat <<'EOF' | ./target/debug/nas --mcp
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"manual","version":"0.0.1"}}}
{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
EOF
```

Cada linha da entrada é uma mensagem JSON-RPC; cada linha da saída,
uma resposta. Nada além do protocolo aparece na stdout — logs vão
pro arquivo de sempre (`logging::init`, o mesmo do `nas-term`), nunca
pra tela.

Pra chamar uma ferramenta, acrescente uma linha `tools/call`:

```json
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"open_file","arguments":{"path":"src/lib.rs"}}}
```

## Ferramentas expostas

Vêm do mesmo registro que `nas-commands` usa internamente
(`Registry::specs()`), filtradas pela marcação `exposed_to_ai` de
cada comando — não existe uma lista separada pra manter em dia. Hoje,
dez comandos: `open_file`, `read`, `search`, `list_files`, `edit`,
`show_diff`, `create_file`, `checkpoint_create`, `checkpoint_restore`
e `save_file`. O esquema de cada um (nome dos argumentos,
obrigatórios, descrição) vem de `schemars`, visível via `tools/list`.

`edit` muda só o buffer deste processo — nada vai pro disco sem
`save_file`, que recusa (pedindo pra reabrir com `open_file`) se o
arquivo mudou no disco desde a última leitura, pra nunca sobrescrever
o trabalho de outro processo.

## Autoria

Toda mudança feita por aqui é journalada como `Author::Ai { agent,
session }` (`docs/decisions/0005` §1): `agent` é o `clientInfo.name`
que o cliente MCP declarou no `initialize`; `session` é gerado uma
vez quando o processo `nas --mcp` sobe. `Ctrl+t a` no `nas-term`
mostra essas edições como de qualquer outro autor.
