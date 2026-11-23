# Configuração do nas

Esta é a referência da configuração em camadas do nas — a decisão
está em `docs/decisions/0007-configuracao-em-camadas.md`; este
documento é só a implementação em dados (camada 1: `config.toml` e
`keymap.toml`), não as razões por trás dela.

Sem nenhum arquivo de configuração, o nas se comporta exatamente como
hoje — nada muda até você criar um dos arquivos abaixo.

## Onde os arquivos ficam

```
$XDG_CONFIG_HOME/nas/config.toml    # opções de comportamento, usuário
$XDG_CONFIG_HOME/nas/keymap.toml    # atalhos, usuário
<raiz-do-projeto>/.nas/config.toml  # opções de comportamento, projeto (restrito)
```

`$XDG_CONFIG_HOME` é `~/.config` quando a variável não está definida
— mesmo fallback manual que `NAS_KEYBOARD_LAYOUT` e o histórico
(`$XDG_STATE_HOME`) já usam. A raiz do projeto é a mesma que o
histórico já identifica: a raiz do repositório Git, se houver um, ou
o diretório onde o nas foi iniciado.

**Não existe `keymap.toml` de projeto.** Atalhos são seus, não do
código que você está editando.

## Ordem de prioridade

```
padrão do nas  <  usuário  <  projeto  <  sessão (ainda não implementada)
```

Cada camada só sobrescreve as opções que ela declara; o resto
continua valendo da camada anterior. Um `.nas/config.toml` com
`tab_width = 2` muda só o `tab_width` — qualquer outra opção que o
seu `~/.config/nas/config.toml` tenha definido continua valendo.

## Configuração recuperável

Cada camada é lida e validada **por inteiro e separadamente** antes
de qualquer valor ser aplicado:

- `config.toml`/`keymap.toml` do usuário são a única camada de
  verdade que a abertura normal do nas tem. Se qualquer um dos dois
  tiver um erro, **nada** é aplicado — a configuração em uso continua
  a mesma de antes (na abertura, o padrão embutido do nas) — e o(s)
  erro(s) aparece(m) na echo area, com arquivo, linha e coluna quando
  fizer sentido.
- `.nas/config.toml` (projeto) é **isolado**: um erro nele nunca
  descarta a configuração do usuário. Só a camada de projeto é
  ignorada (fica valendo padrão + usuário); o erro ainda aparece na
  echo area, pra você saber que ela não foi aplicada.
- **O nas abre normalmente mesmo assim**, nos dois casos. Um erro de
  configuração nunca é motivo pra recusar abrir.

## `config.toml`

### Opções

| Opção | Tipo | Padrão | Vale no projeto? | O que faz |
|---|---|---|---|---|
| `keyboard_layout` | `"qwerty"` \| `"qwerty-abnt"` | `"qwerty"` | não | Layout físico do teclado — mesma opção que a variável de ambiente `NAS_KEYBOARD_LAYOUT` já controla. Se a variável estiver definida, ela vence (é uma troca rápida, só desta sessão); sem ela, vale o que `config.toml` disser. |
| `tab_width` | inteiro, 1 a 16 | `4` | **sim** | Quantas colunas `Tab` insere, quando `indent_with_spaces` for `true`. Com `indent_with_spaces = false` (o padrão), esta opção não tem efeito — `Tab` insere um caractere de tabulação, não espaços. |
| `indent_with_spaces` | booleano | `false` | **sim** | Se `Tab` insere `tab_width` espaços (`true`) ou um caractere de tabulação (`false`, o que o nas já fazia antes desta opção existir). |
| `line_numbers` | booleano | `true` | não | Se a régua de números de linha aparece à esquerda do texto. |

`keyboard_layout` e `line_numbers` não valem em `.nas/config.toml`
(ver "Projeto é restrito" abaixo) — são sobre quem está editando, não
sobre o projeto.

### Projeto é restrito

`.nas/config.toml`, na raiz do projeto, é pra combinar convenções de
formatação entre quem edita o mesmo código — nunca pra mudar como o
nas se comporta como programa:

- nunca executa código — é só dados, como o resto;
- só aceita as opções marcadas "sim" na tabela acima; qualquer outra
  é um **erro de validação** (o arquivo inteiro falha a carregar),
  não um valor ignorado em silêncio;
- nunca muda teclas, permissões ou integrações — não existe
  `keymap.toml` de projeto.

### Erros

Uma chave desconhecida ou um valor fora do esperado aponta o arquivo,
a linha e a coluna, e sugere o nome certo quando for parecido:

```
/home/alguem/.config/nas/config.toml:2:1: chave desconhecida: "tabwidth" (você quis dizer "tab_width"?)
```

Uma opção fora do permitido em `.nas/config.toml`:

```
projeto/.nas/config.toml:1:1: a opção "line_numbers" só vale na configuração do usuário, não no projeto
```

### Exemplo

```toml
# ~/.config/nas/config.toml
keyboard_layout = "qwerty-abnt"
tab_width = 2
indent_with_spaces = true
line_numbers = true
```

```toml
# <projeto>/.nas/config.toml — só as opções marcadas "sim" valem aqui
tab_width = 2
indent_with_spaces = true
```

## `keymap.toml`

Aplicado **por cima** de `crates/nas-keys/keymap/default.toml` —
você só escreve as diferenças, nunca o keymap inteiro. O merge é por
tabela: `[visual]`, `[prefixes.go]`, `[system]`, `[system_prefixes.tempo]`
etc. se combinam uma a uma com a tabela correspondente do padrão.

### Sobrescrever um atalho

Redeclare a tecla com o valor novo — mesmo formato de
`default.toml` (ver `docs/keys.md`):

```toml
[visual]
j = "move-right"   # "j" passa a mover pra direita, em vez de esquerda
```

### Remover um atalho

Uma chave com o valor **`false`** remove o que o padrão declarava
pra aquela tecla, em vez de substituir por outra coisa:

```toml
[visual]
j = false   # "j" não faz mais nada em Visual
```

Isso vale em qualquer tabela de atalhos, incluindo dentro de um
prefixo:

```toml
[prefixes.go]
g = false   # "g g" (início do arquivo) deixa de existir
```

### Validação

Depois do merge, vale a mesma validação que o `default.toml`
embutido já passa: toda tecla precisa ter um formato reconhecido
(ver `docs/keys.md` §2), e todo comando citado (ação, verbo, unidade,
prefixo) precisa existir. Um comando inexistente:

```
/home/alguem/.config/nas/keymap.toml: ação desconhecida no keymap: "voar"
```

### Exemplo

```toml
# ~/.config/nas/keymap.toml
[visual]
j = false            # desliga "j" (mover à esquerda)
z = "move-left"      # e liga no lugar dele

[system_prefixes.tempo]
m = false            # desliga Ctrl+t m (histórico de mensagens)
```

## Recarregar

Salvar `config.toml` ou `keymap.toml` de dentro do próprio nas
recarrega as duas camadas de dados automaticamente — a mesma regra
de tudo ou nada: se o que você acabou de salvar (ou qualquer outra
camada) tiver um erro, a configuração em uso **não muda**, e o erro
aparece na echo area; sem erro, a configuração nova (e o keymap,
se foi o que mudou) entra em vigor na hora, sem precisar reiniciar o
nas.

## `nas --check-config`

Valida `config.toml`/`keymap.toml` (usuário e, se você estiver numa
raiz de projeto, `.nas/config.toml`) sem abrir nenhum terminal —
imprime os erros, se houver, e sai com um código diferente de zero:

```sh
$ nas --check-config
Configuração válida.
$ echo $?
0

$ nas --check-config
/home/alguem/.config/nas/config.toml:3:13: tab_width: esperava um número entre 1 e 16, recebeu 99
$ echo $?
1
```

Útil pra checar antes de editar na mão (ou num hook de pre-commit,
se você versiona sua configuração).

## `nas --no-config`

Abre ignorando `config.toml`, `keymap.toml` e `.nas/config.toml` por
completo — só o padrão embutido do nas, como se nenhum arquivo de
configuração existisse. Nenhum deles é lido, nem validado.

É a saída pra quando a própria configuração deixa o nas inutilizável
— um `keymap.toml` que desligou o atalho que você precisava pra
corrigir o `keymap.toml`, por exemplo. Com `--no-config`, você abre
no padrão, corrige o arquivo com os atalhos de sempre, e na próxima
vez abre normalmente:

```sh
nas --no-config ~/.config/nas/keymap.toml
```
