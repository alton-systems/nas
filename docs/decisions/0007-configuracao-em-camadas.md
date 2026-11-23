# 0007 — configuração em camadas

## Status

Aceita. Implementação em etapas — esta ADR cobre a decisão completa;
só a **camada 1** (dados, TOML) é implementada nesta etapa. A camada
2 (programável, Steel) fica explicitamente pra depois.

## Contexto

O nas hoje não tem nenhuma configuração persistente do usuário: o
keymap é só o embutido (`crates/nas-keys/keymap/default.toml`,
`docs/decisions/0003`), e o único ajuste de comportamento é a
variável de ambiente `NAS_KEYBOARD_LAYOUT` (README). Toda vez que
alguém quer um atalho diferente, precisaria editar o binário. Esta
ADR registra como isso passa a ser configurável — por dados (TOML),
hoje, e por um programa (Steel) mais adiante — sem abrir uma porta
que a ADR 0006 (permissões por efeito, autoria) depois teria que
fechar.

## Decisão

### 1. Duas camadas

- **Dados (TOML), agora**: `config.toml` (opções de comportamento) e
  `keymap.toml` (atalhos) em `$XDG_CONFIG_HOME/nas/` — padrão
  `~/.config/nas/`, mesmo fallback manual que `NAS_KEYBOARD_LAYOUT` e
  o `$XDG_STATE_HOME` do histórico já usam quando a variável não
  existe (ADR 0005 §5). Puro dado, nunca código — mesmo princípio de
  `docs/keys.md`/`default.toml` (ADR 0003): atalhos e opções vivem
  como dados declarativos, nunca espalhados pelo código Rust.
- **Programável (Steel), depois**: `init.scm`, só **depois** que
  `nas-keys` e `nas-commands` se unificarem num registro de comandos
  só (ADR 0006 §1, etapa 3g) — um `init.scm` escrito contra dois
  sistemas de comando separados ficaria obsoleto no dia em que
  unificassem; não faz sentido construí-lo duas vezes. Ver §6.

### 2. Ordem de prioridade

**Padrão do nas < usuário < projeto < sessão.** Cada camada só
sobrescreve o que declara; o que não toca continua valendo da camada
anterior.

- **Padrão**: os valores embutidos no binário — o que o nas já faz
  hoje sem nenhum arquivo de configuração.
- **Usuário**: `~/.config/nas/{config,keymap}.toml`.
- **Projeto**: `.nas/config.toml` na raiz do projeto (mesma raiz que
  `nas-history` já identifica, ADR 0005 §6) — restrito, ver §4. **Não
  existe `keymap.toml` de projeto** (§4): atalhos são do usuário, não
  do código que ele está editando.
- **Sessão**: mudanças feitas em tempo de execução (ainda sem tecla
  nem comando que as produza), descartadas ao sair — **ainda não
  implementada**; esta etapa só reserva o lugar dela na ordem, pra
  que a camada de dados já nasça sabendo que vai ter algo por cima.

### 3. Configuração recuperável

O nas nunca abre com uma configuração quebrada, e um erro de
configuração nunca é a razão de não abrir:

- **Ler, validar tudo e só então aplicar de uma vez.** Todo arquivo
  de toda camada é lido e validado antes de qualquer valor ser
  aplicado — tudo ou nada, mesmo princípio que a ADR 0005 já usa pra
  restaurar um checkpoint (nenhuma escrita parcial).
- **Com erro, mantém a última configuração válida.** Na abertura, "a
  última válida" é o padrão embutido; depois de recarregar (§
  "Recarregar" na Parte 2), é a que já estava rodando. O erro aparece
  na echo area (docs/keys.md regra 8) — nunca só no log.
- **Um erro de configuração nunca impede o nas de abrir** — mesmo
  comportamento que um arquivo não-UTF8 já tem hoje (`Document::open`
  recusa e abre vazio, nunca recusa o processo inteiro).
- **Camadas isoladas: cada uma é validada por inteiro e
  separadamente** — um erro numa camada nunca descarta outra já
  válida. Em particular, `.nas/config.toml` (projeto) é a única
  camada que não é fatal: se ele tiver um erro, só a camada de
  projeto é ignorada — padrão e usuário continuam aplicados, o erro
  aparece na echo area, e o nas abre com a configuração do usuário
  intacta. `config.toml`/`keymap.toml` do usuário, por outro lado,
  continuam fatais como no parágrafo anterior: é a única camada de
  verdade que a abertura normal tem, e aplicar metade dela seria
  pior que cair no padrão embutido.

### 4. Projeto é restrito

`.nas/config.toml` existe pra que um projeto compartilhado combine
convenções de formatação entre quem o edita — nunca pra executar
nada nem pra mudar como o nas se comporta como programa:

- **Nunca executa código.** Só dados — a mesma razão (ADR 0003) que
  `default.toml` nunca é um script.
- **Só aceita as opções marcadas como permitidas por projeto**
  (indentação, formatação e afins — ver a tabela única de opções na
  Parte 2). Uma opção não marcada, se aparecer em `.nas/config.toml`,
  é um **erro de validação** (§3), não um valor ignorado em silêncio.
- **Nunca muda teclas, permissões, integrações ou qualquer coisa de
  segurança** — por isso não existe `keymap.toml` de projeto (§2), e
  por isso a lista de opções permitidas por projeto é pequena e
  explícita, nunca "tudo que não for claramente perigoso".

### 5. Expor um comando à IA é explícito, por comando

Estar no registro de `nas-commands` (ADR 0006 §1) não torna um
comando — nativo ou escrito pelo usuário (§6) — visível pra um
agente. Cada comando precisa de uma marca própria, explícita, pra
aparecer como ferramenta MCP (ADR 0006 §3). Hoje isso não muda nada
na prática (ADR 0006 ainda não tem comandos de usuário pra marcar),
mas a regra já fica registrada: quando o Steel chegar (§6) e alguém
escrever seu próprio comando, ele é privado por padrão — expor à IA
é um passo extra, nunca o padrão.

### 6. Para o Steel, no futuro

Três condições que a camada 1 (dados) já precisa deixar prontas, pra
quando a camada 2 (programável) chegar:

- **O registro de comandos precisa aceitar comandos registrados em
  tempo de execução** — hoje `nas-commands::Registry` só recebe os
  comandos nativos, registrados uma vez no início (`register_all`,
  ADR 0006 §1); precisa aceitar também os que `init.scm` declarar,
  carregado depois.
- **`init.scm` usa os mesmos efeitos de `nas-commands`** (`Pure`,
  `ReadFile`, `WriteBuffer`, `WriteHistory`, `WriteFile`, `Shell`,
  `Network` — ADR 0006 §1/§5) — um comando em Scheme declara seu
  efeito do mesmo jeito que um nativo, não um sistema de permissão
  paralelo.
- **O interpretador só entrega a um comando as funções dos efeitos
  que ele declarou** — um comando que declara `Pure` nunca tem acesso
  a uma função que escreve em disco, mesmo que o Steel embutido,
  tecnicamente, pudesse chamá-la; a sandbox é por capacidade
  entregue, não por confiança no que o script promete.

### 7. A IA só altera configuração por um comando próprio

Nunca escrevendo `config.toml`/`keymap.toml` diretamente — um comando
de `nas-commands` dedicado (ex. `set_config`), com efeito que pede
aprovação (ADR 0006 §5: não é `Pure` nem `WriteBuffer`, é configuração
do nas como programa, mais parecido com `WriteFile`), que passa pela
mesma validação da §3 antes de aplicar. Isso é consequência direta de
§5: mesmo que a IA tivesse acesso de escrita ao arquivo por fora do
nas (um `Shell` aprovado, por exemplo), a trilha de autoria (ADR 0006
§6) só existe pra mudanças que passam por um comando do registro —
escrever o arquivo na mão seria invisível pra `Ctrl+t a`.

## Fora de escopo desta etapa

- **Steel/`init.scm`** (§1/§6) — depois da unificação de comandos
  (ADR 0006, etapa 3g).
- **Temas** — não inventados aqui; as três opções da Parte 2 são
  comportamento que já existe, não aparência.
- **A camada de sessão** (§2) — só o lugar dela na ordem está
  reservado; nenhuma tecla/comando a produz ainda.
- **Expor configuração pra IA, e qualquer comando de IA novo** (§5/§7)
  — inclusive `set_config`: registrado como decisão, não
  implementado nesta etapa.

## Consequências

- Nova crate `nas-config` (justificada na Parte 2 do PR) — carrega e
  valida as duas camadas de dados; `nas-term` passa a chamá-la no
  início, do mesmo jeito que já chama `nas-history`/`nas-keys`.
- `nas-keys` não muda sua API pública: `keymap.toml` do usuário é
  aplicado por cima de `DEFAULT_TOML` **antes** de chamar
  `Keymap::from_toml_str`, reaproveitando toda a validação que já
  existe ali (tecla válida, comando existente) em vez de duplicá-la.
- `App` ganha um campo de configuração resolvida e um caminho de
  recarregar ao salvar `config.toml`/`keymap.toml` de dentro do nas —
  a mesma regra de tudo-ou-nada da §3 vale pro recarregamento.
- `nas --check-config` é a primeira vez que o binário faz algo além
  de abrir o editor — sai antes de montar qualquer terminal.
