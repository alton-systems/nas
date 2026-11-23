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
- **Sem aprovação de verdade (até a etapa 3d).** Qualquer comando com
  efeito `WriteFile`/`Shell`/`Network` é recusado por padrão, com um
  erro dizendo que a aprovação chega na etapa 3d. `--approve-writes`
  (abaixo) é a única válvula de escape — temporária, só pra testar.
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

Pra liberar `save_file` enquanto a etapa 3d (aprovação de verdade)
não existe:

```sh
claude mcp add nas -- /caminho/para/nas --mcp --approve-writes
```

O servidor descobre a raiz do projeto do mesmo jeito que o histórico
(`docs/decisions/0005` §6: raiz do Git, ou o diretório onde foi
iniciado) — registre o comando a partir do diretório do projeto, ou
confira que o Claude Code inicia o subprocesso lá.

Confirme com `claude mcp list` — o nas deve aparecer como `connected`
depois da primeira chamada.

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
