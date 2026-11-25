# As teclas do nas

Esta é a especificação do sistema de teclas do nas. A partir deste
documento, **a especificação vive aqui**, não no pacote de Emacs
[nas-keys](https://github.com/2gab/nas-keys) — ver
`docs/decisions/0004-especificacao-de-teclas-no-nas.md`. O nas-keys
continua existindo, mas passa a ser atualizado a partir deste arquivo,
não o contrário.

A implementação em dados é `crates/nas-keys/keymap/default.toml`; a
mecânica de como um estado, prefixo, verbo e unidade funcionam é
`crates/nas-keys/src/machine.rs`. Nenhuma tecla deveria estar
hardcoded em Rust — se você acha uma, é um bug nesta separação.

## 1. A ideia central

Na escrita chinesa, um ideograma é formado por **radicais**, partes
pequenas com significado próprio. Quem conhece os radicais entende um
ideograma que nunca viu.

O nas aplica isso ao teclado:

- **Cada tecla é um radical** com **um significado só**, que vale em
  qualquer contexto.
- **Combinar teclas forma compostos.** Quem conhece as peças entende a
  combinação sem decorar.
- **O modificador (Ctrl, Alt, Shift) define a classe** do significado,
  como o radical que indica se um ideograma fala de água ou de fogo.

Exemplo: `;` é "direita". `:` (Shift+`;`) é "o extremo da direita", o
fim da linha. `d` é "apagar". Logo, `d :` é "apagar até o fim da
linha", e ninguém precisa decorar isso — quem já sabe o que `;` e `:`
significam, e o que `d` significa, já sabe o que `d :` faz.

## 2. Glossário

- **Radical**: uma tecla com um significado fixo (ex.: `;` = direita).
- **Classe**: o que o modificador (ou a ausência dele) indica sobre
  *onde* o radical age — texto, intensidade, alternância (entre duas
  coisas) ou sistema (o editor como programa). Ver §3.
- **Verbo**: uma tecla que inicia uma ação sobre uma região de texto
  (`d` apagar, `c` copiar, `x` recortar). Espera um **objeto**, a
  menos que já exista uma seleção ativa.
- **Objeto**: o que vem depois de um verbo, definindo a região sobre a
  qual ele age — outro radical de movimento, o próprio verbo repetido,
  ou "o arquivo inteiro". Ver §4.3.
- **Unidade**: uma tecla que, sozinha, espera uma direção para
  completar um movimento maior que um caractere ou uma linha — `w`
  (palavra) e `W`, Shift+`w` (parágrafo, a mesma unidade intensificada
  pela regra 3). Funciona tanto sozinha (movimento puro)
  quanto depois de um verbo (formando um objeto composto, ex. `d w ;`).
- **Composto**: a sequência completa de teclas até ela resolver numa
  ação — `d ;`, `d w ;`, `g l`, `SPC f` são todos compostos.
- **Prefixo**: uma tecla que, sozinha, não faz nada e espera mais uma
  tecla pra escolher entre um conjunto fechado de ações — `g` (ir
  para) e `Espaço` (líder).
- **Frase**: uma sequência de compostos digitada no estado Sequential,
  mostrada antes de executar — é o que a IA vai propor, no marco 4.
- **Estado**: Insert, Visual, Sequential ou Review — ver §5.4 e §5.10.

## 3. As regras (o contrato do sistema)

1. **Um radical, um significado.** Se `k` é "para baixo", ele é "para
   baixo" sozinho, depois de um verbo, e em qualquer prefixo.
2. **O modificador define a classe.** A mesma tecla em outra classe
   mantém a ideia (a direção, por exemplo) e muda o "mundo" onde ela
   age: texto, alternância ou sistema.

   `Espaço` é o radical de **comando**: falar com o editor, em vez de
   escrever texto nele. Sozinho, abre o líder — um comando (`Espaço f`
   busca, por exemplo). Dobrado (`Espaço Espaço`), entra no
   Sequential — uma frase inteira de comandos. Com Alt (`Alt+Espaço`),
   alterna entre escrever texto e dar comandos (Insert ↔ Visual) — o
   exemplo central da classe alternar (§4). `Espaço t` fica reservado
   para falar com a IA (a "tia"); `Shift+Espaço` abre a paleta de
   comandos, só no Visual — essa combinação depende do protocolo de
   teclado do Kitty pra distinguir de um `Espaço` comum (sem ele,
   continua sendo só o líder). `Espaço p`, dentro do próprio líder,
   abre a mesma paleta em qualquer terminal (ver §5.9).
3. **Shift intensifica.** Leva o significado ao extremo — tanto a
   *distância* de uma direção (`j` → `J`: um caractere → o início da
   linha) quanto o *tamanho* de uma unidade (`w` → `W`: palavra →
   parágrafo). As duas seguem a mesma escala, cada degrau maior que o
   anterior: caractere → palavra → parágrafo → extremos (linha/
   arquivo).
4. **Verbo + objeto.** Todo verbo aceita os mesmos objetos.
5. **Seleção primeiro.** Com uma seleção ativa, o verbo age nela na
   hora, sem pedir objeto.
6. **Posição antes de letra.** As teclas são definidas pela posição
   física no qwerty e traduzidas para o layout real. No ABNT2, o `;`
   do qwerty é o `ç`, e o `:` é `Shift+ç`.
7. **Tudo é um comando com nome e descrição.** Toda ação tem um nome
   (usado no keymap) e uma descrição curta (usada nas dicas) — ver
   `[commands]` em `default.toml`.
8. **Descobrível.** Depois de um verbo, de um prefixo ou de uma
   unidade, o editor mostra quais teclas podem vir em seguida e o que
   cada uma faz — na **echo area**, uma linha separada logo abaixo da
   barra de status, nunca na própria barra.

   A barra de status mostra só o estado **persistente** (estado
   modal, nome do buffer, `[+]`, posição do cursor) e nunca é
   substituída por nada — mensagens, prompts (buscar, abrir arquivo,
   trocar de buffer, ir para linha, nome do checkpoint, restaurar,
   confirmações) e a dica de tecla pendente são todos **passageiros**,
   e vivem só na echo area, por baixo. A echo area fica vazia (uma
   linha em branco) quando não há nada a dizer; uma mensagem some por
   conta própria depois de alguns segundos, ou na tecla seguinte,
   qualquer uma — nunca espera um Enter. Uma mensagem maior que a
   largura da tela é cortada com "…"; a versão inteira fica num
   histórico de mensagens consultável (como o `*Messages*` do Emacs).

   A dica em si segue o estilo *which-key*: cabendo numa linha só
   (`rótulo  »  tecla descrição  ·  tecla descrição  ...`), é só isso;
   quando não cabe, a echo area **cresce temporariamente** — o rótulo
   isolado na primeira linha, as opções em colunas nas seguintes, sem
   cortar nenhuma — e volta a uma linha só na tecla seguinte (ou Esc).

## 4. As classes

| Classe | Como se aciona | Significado | Estados em que vale |
|---|---|---|---|
| Texto | tecla sem modificador | mover e editar o texto | Visual |
| Intensidade | Shift | o mesmo significado, no extremo | Visual |
| Alternar | Alt | trocar entre duas coisas — texto e comando (`Alt+Espaço`), ou janelas (`Alt+direção`, reservado) | todos |
| Sistema | Ctrl | o editor como programa: arquivos, buffers, sessão | todos |
| Ir para | prefixo `g` | saltar para um destino | Visual |
| Líder | prefixo `Espaço` | comandos de alto nível sobre o texto | Visual |

Para decidir onde uma ação nova entra: mexe no **texto**, sem
modificador; **alterna** entre duas coisas, Alt; mexe no **editor como
programa**, Ctrl; é um **comando de alto nível**, líder.

## 5. A especificação completa das teclas

As teclas abaixo estão na posição qwerty. Onde nada é dito, o
comportamento é o que `default.toml` implementa.

### 5.1 Movimentos (estado Visual)

| Teclas | Movimento |
|---|---|
| `j` / `;` | caractere anterior / próximo |
| `k` / `l` | próxima linha / linha anterior |
| `J` / `:` | início / fim da linha |
| `K` / `L` | fim / início do arquivo |
| `w j` / `w ;` | palavra anterior / próxima palavra |
| `W k` / `W l` | próximo parágrafo / parágrafo anterior |

`w` é um radical de unidade: sozinho, não faz nada e espera uma
direção. `W` (Shift+`w`) é a mesma unidade, intensificada pela regra 3
— parágrafo em vez de palavra, a próxima escala. Uma direção sem
sentido para a unidade (ex.: `w k`) não faz nada — a unidade é
cancelada, como um prefixo. `p` sozinho (sem Shift) é reservado e não
faz nada — ver §6.

### 5.2 Verbos e ações (estado Visual)

| Tecla | Significado | Sem seleção | Com seleção |
|---|---|---|---|
| `d` | apagar | espera um objeto | apaga a seleção |
| `c` | copiar | espera um objeto | copia a seleção |
| `x` | recortar | espera um objeto | recorta a seleção |
| `s` | selecionar | marca o início da seleção; mover estende | marca de novo no cursor |
| `S` | selecionar tudo | | |
| `y` | colar | age na hora | substitui a seleção |
| `u` | desfazer | age na hora | |
| `.` | repetir | repete o último **composto que alterou o texto** (ex.: `d w ;`) na posição atual do cursor | |

Depois de um verbo agir sobre uma seleção, a seleção é desfeita.

Com um objeto (não uma seleção), `d` e `x` sempre copiam a região pra
área de transferência antes de apagar — como `kill-line`/`kill-word`
no Emacs. Já `d`/`x` **com uma seleção** não copiam
(`DeleteSelection`) — só `x` com seleção copia (`Cut`); a tabela acima
reflete essa diferença: a célula "com seleção" de `d` não fala de área
de transferência, a de `x` fala ("recorta").

### 5.3 Objetos (o que vem depois de um verbo)

| Objeto | Região |
|---|---|
| qualquer movimento **horizontal** (`j`, `;`, `J`, `:`, `w j`, `w ;`) | do cursor até o destino do movimento, por caractere. Pra frente, o caractere de destino **não** entra: `d ;` apaga exatamente um caractere; `d w ;` apaga até o início da próxima palavra; `d :` apaga até o fim da linha, sem a quebra de linha |
| qualquer movimento **vertical** (`k`, `l`, `K`, `L`, `W k`, `W l`) | por linha: linhas inteiras, da linha do cursor até a linha de destino, inclusive. `d k` apaga a linha atual e a de baixo; `d K` apaga da linha atual até o fim do arquivo |
| o próprio verbo repetido (`d d`, `c c`, `x x`) | a linha atual inteira, com a quebra de linha |
| `a` | o arquivo inteiro. `d a` e `x a` pedem confirmação (a mesma combinação repetida confirma) e, como qualquer outro objeto de `d`/`x`, copiam o conteúdo pra área de transferência antes de apagar |

Uma tecla que não é objeto válido depois de um verbo **não executa
nada**: o editor mostra a dica dos objetos possíveis e **continua
esperando** — ao contrário de um prefixo (§5.5/§5.6), que cancela numa
tecla inválida, um verbo pendente persiste até um objeto válido ou
`Esc`. O mesmo vale pra uma unidade já iniciada por um verbo (`d w` à
espera de `j`/`;`): ainda estamos resolvendo o objeto do verbo. Uma
unidade **sozinha** (sem verbo, §5.1) cancela numa direção inválida,
como um prefixo — não há verbo em jogo. `Esc` cancela qualquer verbo,
prefixo ou unidade pendente, em qualquer um dos dois casos. Sem nada
pendente, `Esc` desfaz uma seleção ativa, sem apagar nada; sem
pendente nem seleção, não faz nada — um cancelamento de cada vez.

Os objetos horizontal e vertical não são declarados numa tabela
separada — são **derivados** das próprias ações de movimento já
ligadas em `[visual]` (§5.1): cada movimento diz, em `machine.rs`, se
é horizontal ou vertical, e a mesma tecla que já move o cursor assim
também serve de objeto. Mudar a tecla de `j`/`;`/`J`/`:`/`k`/`l`/`K`/`L`
em `[visual]` muda o objeto correspondente automaticamente, sem editar
dois lugares.

### 5.4 Estados

| Tecla | Efeito |
|---|---|
| `Enter` (Visual) | entra no Insert |
| `Alt+Espaço` | alterna entre Insert e Visual — o exemplo central da classe alternar (§4), acessível em qualquer estado |
| `Espaço Espaço` | entra no Sequential |
| `Enter` / `Esc` / `Backspace` (Sequential) | executa a frase / cancela / apaga a última tecla |

### 5.5 Prefixo `g` (ir para)

| Teclas | Destino |
|---|---|
| `g g` | início do arquivo |
| `g l` | uma linha (pede o número) |
| `g h` | o trecho atual, na revisão — "ir para" (`g`) o lugar d`o` trecho (`h`, docs/decisions/0008-modo-de-revisao.md §4); global, mas sem efeito fora da revisão (não há trecho atual) |
| `g c` | uma posição em caracteres (reservado, não implementado) |
| `g f` | a definição de uma função (reservado, depende de LSP) |

### 5.6 Prefixo `Espaço` (líder)

| Teclas | Ação |
|---|---|
| `Espaço f` | buscar |
| `Espaço p` | paleta de comandos (ver §5.9) |
| `Espaço r` | substituir (reservado, não implementado — ver §7) |
| `Espaço h` | ajuda (reservado, não implementado — ver §7) |
| `Espaço Espaço` | entrar no Sequential |

Os movimentos por palavra e parágrafo **não** ficam no líder — são a
unidade `w`/`W` (§5.1).

### 5.7 Classe sistema (Ctrl, em todos os estados)

| Atalho | Ação |
|---|---|
| `Ctrl+s` | salvar |
| `Ctrl+q` | sair do editor (pergunta se houver alterações não salvas, em qualquer buffer) |
| `Ctrl+o` | abrir arquivo (num buffer novo, ou troca pro já aberto) |
| `Ctrl+b` | trocar de buffer |
| `Ctrl+k` | fechar o buffer atual (pergunta se tiver alterações não salvas; fechar o único buffer aberto deixa um vazio no lugar) |
| `Ctrl+r` | recarregar o buffer atual a partir do disco (pergunta se tiver alterações não salvas) |
| `Ctrl+t` | prefixo "tempo" — histórico e checkpoints (tabela abaixo) |

`o` e `b` não são mais comandos do Visual — viraram `Ctrl+o` e
`Ctrl+b`, porque abrir arquivo e trocar de buffer são o editor agindo
como *programa*, não uma edição de texto.

`Ctrl+r` também resolve uma mudança externa detectada (o arquivo
mudou no disco desde a última leitura/escrita do nas — `git
checkout`, um formatador rodando fora do nas, outro programa editando
o mesmo arquivo): o nas avisa na echo area (regra 8), sem recarregar
sozinho, e `Ctrl+r` é quem recarrega de propósito. `Ctrl+s` nunca
sobrescreve uma mudança externa sem perguntar — pede a mesma
confirmação (repetir a tecla) de `Ctrl+q`/`Ctrl+k`. Ver
docs/decisions/0005-historico-com-autoria.md §1.

Esta tabela é dados: vive em `[system]`, em `default.toml`, com a
descrição de cada comando em `[commands]`. `nas-term` só sabe
perguntar `Machine::system_action(tecla)` — nenhum atalho desta classe
está escrito direto no código do nas-term.

**Prefixo `Ctrl+t` ("tempo"):** histórico com autoria e checkpoints
do projeto (docs/decisions/0005-historico-com-autoria.md). A tecla
seguinte, **sem** Ctrl, escolhe o comando — mesma ideia do líder
(§5.6), só que dentro da classe sistema, em `[system_prefixes.tempo]`:

| Atalho | Ação | Por quê esta letra |
|---|---|---|
| `Ctrl+t n` | novo checkpoint nomeado do projeto | `n`ovo, livre |
| `Ctrl+t l` | listar checkpoints | `l`istar, livre |
| `Ctrl+t e` | diff entre os dois checkpoints mais recentes | `d` já é apagar (verbo); `e`ntre as duas versões |
| `Ctrl+t r` | restaurar um checkpoint | `r`estaurar, livre |
| `Ctrl+t a` | autoria — histórico de edições do arquivo atual | `b` já é `Ctrl+b` (buffer); `a`utoria |
| `Ctrl+t m` | histórico de mensagens (echo area) num overlay | `m`ensagens, livre |
| `Ctrl+t v` | revisar — abre direto, no checkpoint padrão (§5.10) | `v`er/revisar, livre |
| `Ctrl+t V` | revisar — abre a lista de checkpoints pra escolher | Shift leva a ação ao extremo (regra 3): em vez do caminho direto, mostra **todas** as opções antes de decidir |
| `Ctrl+t z` | apagar o histórico do projeto (exige digitar "apagar") | sem uso melhor por perto; ação rara e destrutiva, letra "longe" das outras |

`Ctrl+t` por si só não faz nada além de abrir o prefixo — uma tecla
que não esteja na tabela cancela, como qualquer outro prefixo (§3,
regra 4) e volta a resolver normalmente.

Não use combinações de Ctrl que cheguem iguais a outras teclas no
terminal: `Ctrl+i` (Tab), `Ctrl+m` e `Ctrl+j` (Enter), `Ctrl+[` (Esc),
`Ctrl+h` (Backspace em alguns terminais).

### 5.8 Classe alternar (Alt, em todos os estados)

Alt é a classe de **alternar entre duas coisas** — não só janelas.
`Alt+Espaço` (§5.4) é o exemplo central: alterna entre escrever texto
e dar comandos. `Alt+direção` fica reservado pra alternar entre
janelas, a mesma ideia aplicada à tela:

| Atalho | Ação |
|---|---|
| `Alt+Espaço` | alterna entre Insert e Visual (texto ↔ comando) |
| `Alt+j` / `Alt+;` / `Alt+k` / `Alt+l` | ir para a janela à esquerda / direita / abaixo / acima (reservado) |

O nas ainda não tem janelas: `Alt+direção` fica só documentado — ver
§7. Ver também §10 (decisão em aberto sobre `Ctrl+b`).

### 5.9 Paleta de comandos (Visual)

| Atalho | Ação |
|---|---|
| `Shift+Espaço` | abre a paleta de comandos — só no Visual; depende do protocolo de teclado do Kitty (`supports_keyboard_enhancement`) pra distinguir de um `Espaço` comum |
| `Espaço p` | abre a mesma paleta, em qualquer terminal — a entrada que funciona sem o protocolo do Kitty |

A lista é **todo** nome de `[commands]` do keymap (já com o keymap do
usuário aplicado) — não só os que têm um atalho. Um comando sem
atalho (por natureza, como "a linha atual" — só existe como "o verbo
repetido", nunca uma tecla própria; ou porque o usuário removeu a
única tecla dele em `keymap.toml`, `chave = false`, docs/config.md)
aparece com a coluna da tecla vazia, e `Enter` ainda executa: como
não há tecla pra repetir, a paleta despacha a ação diretamente. Um
comando destrutivo (ex.: apagar o histórico do projeto) usa a mesma
cor de alerta que o which-key já usa pra isso, com um `!` de prefixo.

Com a paleta aberta: `↑`/`↓` movem a seleção na lista (dando a volta
nas duas pontas), `Enter` executa o comando selecionado — com atalho,
exatamente como apertar a tecla dele, incluindo abrir um prompt se o
comando pedir algo (ex.: "ir para uma linha") ou armar um verbo/
unidade pendente que uma tecla seguinte completa normalmente (e que
`.` continua repetindo depois, como sempre) — e `Esc` fecha sem
executar nada. Qualquer outra tecla normal entra na busca (aproximada,
por nome ou descrição, sem diferenciar acentos nem maiúsculas);
nenhuma tecla chega à máquina modal enquanto a paleta está ativa, do
mesmo jeito que um prompt (regra 8, "a dica which-key segue a mesma
ideia de camada por cima da barra de status").

Em `Espaço`, no líder, `p` é livre pra isto: diferente do `p`
sozinho em `[visual]` (reservado pra "dentro de"/"em volta de", §6),
o namespace do líder é independente, e nada mais usa essa letra ali.

### 5.10 Estado `Review` — o modo de revisão

Primeiro estado além de Insert/Visual/Sequential (§2/§5.4) — um
buffer somente leitura com os trechos alterados de uma sessão de IA
(ou de outro checkpoint escolhido), um por arquivo tocado
(docs/decisions/0008-modo-de-revisao.md). Aberto por `Ctrl+t v`/
`Ctrl+t V` (§5.7), nunca por uma tecla do próprio Visual.

**Mecanismo: uma camada sobre o `[visual]` resolvido, não uma cópia
dele.** `[review]`, em `default.toml`, só declara o que é novo pra
este estado; o que ela **não** menciona cai no `[visual]` já
resolvido — com o `keymap.toml` do usuário por cima, não o
`default.toml` cru. Isso é o que faz um movimento remapeado pelo
usuário (`j`/`k`/`l`/`;`/`J`/`:`/`K`/`L`, docs/config.md) continuar
valendo dentro da revisão sem que `[review]` precise saber que foi
remapeado.

Dessa queda ficam de fora, **bloqueadas por nome de ação** (nunca por
tecla física — um movimento ou verbo remapeado continua bloqueado
pelo nome, não pela tecla original): qualquer verbo
(`delete`/`copy`/`cut`), `start-selection`, `select-all`, `paste` e
`repeat` — nenhum faz sentido num buffer que não se edita. Uma tecla
bloqueada avisa na echo area, nunca falha em silêncio.

| Teclas | Ação | Por quê, contra as regras do §3 |
|---|---|---|
| `h k` / `h l` | próximo / trecho anterior | `h` é uma unidade nova — "trecho alterado" — no mesmo mecanismo de `w`/`W` (§5.1): a direção usa as mesmas teclas que já significam "abaixo"/"acima", igual a `W k`/`W l` pra parágrafo; nenhum radical de movimento é redefinido (regra 1) |
| `H k` / `H l` | próximo / arquivo anterior | `H` (Shift de `h`) escala trecho → arquivo — regra 3, o mesmo salto de escala que leva palavra a parágrafo; só existe em `[review]`, porque "mais de um arquivo" só faz sentido aqui |
| `Enter` | aceita o trecho sob o cursor (marca como revisado, persiste) | reaproveita o que `Enter` já significa em todo outro estado — avançar/confirmar o que está pendente (Visual: entra no Insert; Sequential: executa a frase) |
| `u` | rejeita o trecho sob o cursor (desfaz, só no arquivo real, se o texto ainda for o que a IA deixou) | não é uma tecla nova: é o `u` de sempre (desfazer), com o alvo "o trecho atual" em vez de "a última transação" — mesmo significado, alvo diferente |
| `g h` | abre o arquivo real na linha do trecho, sai da revisão | vive em `[prefixes.go]` (§5.5), não em `[review]` — compõe "ir para" (`g`) com "trecho" (`h`), por isso continua global |
| `Esc` | sai da revisão, volta pro Visual | um degrau a mais na mesma cascata de `Esc` de sempre (cancela pendente; sem pendente, desfaz seleção; sem seleção, isto) |

`h` fica fixado como um radical do nas **inteiro**, não só da
revisão — "trecho alterado" — mesmo que hoje só `[review]` de fato
ligue algo a ele (ver §6). `H` não generaliza: "o próximo arquivo" só
faz sentido com mais de um arquivo em jogo, o que só a revisão tem.

**Depois de `u` com sucesso, a echo area diz se a rejeição já está no
disco ou só pendente** — nunca uma mensagem genérica: "Trecho
rejeitado e salvo." quando não havia buffer aberto pro arquivo (um é
aberto só pra isto e salvo na hora); "Trecho rejeitado; arquivo com
alterações não salvas." quando já havia um buffer aberto (a rejeição
entra ali, sem salvar sozinho — o usuário decide quando salvar, como
qualquer outra edição dele).

**Um trecho pode ser "arquivo removido"** — o arquivo existia no
checkpoint e não existe mais (nem buffer, nem disco): aparece como um
trecho só, com o conteúdo antigo inteiro do lado `-`. `Enter` aceita
normalmente; `u` recria o arquivo com o conteúdo do checkpoint (mesma
mensagem salvo/pendente acima); `g h` não tem arquivo real pra abrir
— avisa em vez de abrir um buffer vazio numa linha que não existe.

A dica which-key de `[review]` é **persistente** na echo area
enquanto o estado for `Review` — ao contrário da dica de um
verbo/prefixo/unidade pendente (regra 8), que só aparece até a tecla
seguinte. A barra de status mostra `[Revisão]` no lugar de `[Visual]`/
`[Insert]`.

A paleta de comandos (§5.9) passa a filtrar pelo estado atual: os
comandos que só existem em `[review]` (aceitar, navegar por trecho/
arquivo) só aparecem na paleta dentro da revisão — fora dela não há
trecho nenhum pra agir sobre. `goto-hunk` (`g h`), por viver em
`[prefixes.go]`, continua global, em qualquer estado.

**Relação com a issue #9** (modos: contexto + capacidades): o
mecanismo de camada acima — `[review]` consultada primeiro, bloqueio
por nome de ação, o resto cai no keymap resolvido — é o primeiro
código real de "uma camada de teclas sobre outra" no nas, não só uma
analogia. Não generaliza (só existe esta camada, o bloqueio é uma
lista fixa em Rust, não há resolução de conflito) — isso fica pra
quando a issue #9 chegar, trocando esta camada única por N camadas
declaradas como dados.

## 6. Teclas reservadas (não implementadas)

| Teclas | Reservadas para |
|---|---|
| `i` / `o` | objetos "dentro de" e "em volta de" (como o `i"` e o `a(` do Vim) |
| `p` | sem uso — a unidade parágrafo é `W` (Shift+`w`), não `p` |
| `h k` / `h l` | em buffers comuns (fora da revisão): um futuro "próxima/anterior mudança" — `h` já significa "trecho alterado" (§5.10), contra o checkpoint mais recente, presumivelmente; não decidido, só reservado |
| números `0`–`9` | contagem antes de um comando (ex.: `3 w ;`) |
| `m` | prefixo livre para atalhos do próprio usuário, vazio por padrão |
| `g c` | ir para uma posição em caracteres |
| `g f` | ir para a definição de uma função (LSP) |
| `Espaço r` | substituir (query-replace) |
| `Espaço h` | ajuda |
| `Espaço t` | falar com a IA (a "tia") |
| `Alt+j`/`;`/`k`/`l` | gerência de janelas |

## 7. O que não está implementado, e por quê

- **`Espaço r` (substituir):** precisa de um fluxo interativo de
  confirmação por ocorrência (como `query-replace` do Emacs) — maior
  que uma linha de entrada simples. Fica pra quando esse fluxo
  existir.
- **`Espaço h` (ajuda):** a paleta de comandos (§5.9) e as dicas de
  verbo/prefixo/unidade (regra 8) já cobrem boa parte da descoberta
  por busca — o que falta é uma ajuda mais rica (docstring completa,
  exemplos), além do nome e da descrição curta que a paleta já
  mostra.
- **Gerência de janelas (classe alternar, `Alt+direção`):** `nas-term`
  não tem múltiplos painéis — só um buffer visível por vez (trocar de
  buffer não divide a tela).
- **`Espaço t` (falar com a IA):** não há IA integrada neste marco
  (ver README, Marco 4) — não tem pra onde apontar ainda, igual
  `Espaço h`.
- **`g c` / `g f`:** `g c` (posição em caracteres) não tem um caso de
  uso claro ainda; `g f` depende de um cliente LSP, que não existe
  neste marco.
- **Objetos "dentro de"/"em volta de" (`i`/`o`) e contagem numérica:**
  reservados deliberadamente — ver §6. Adicioná-los aumentaria a
  gramática do verbo+objeto sem necessidade comprovada ainda.

## 8. O que aprendemos com cada referência

- **Vim:** a gramática verbo + movimento (`d` + `w` = apagar uma
  palavra) é a ideia central deste redesenho — um pequeno número de
  verbos que aceitam o mesmo conjunto de movimentos como objeto, em
  vez de um atalho por combinação. `.` (repetir o último comando que
  alterou o texto) vem de lá também.
- **Emacs:** comandos com nome e docstring (regra 7), prefixos de duas
  teclas (`C-x` no Emacs real; `g`/Espaço aqui) e `C-g` como cancelar
  universal — nosso `Esc` cancela verbo, prefixo, unidade ou (sem nada
  pendente) uma seleção ativa, do mesmo jeito. `kill-line`/`kill-ring`
  também inspiram "apagar copia pra área de transferência".
- **VS Code:** a paleta de comandos (todo comando é descobrível por
  nome, não só por atalho) e atalhos que mudam de significado por
  contexto — aqui, o contexto é o estado (Visual/Insert/Sequential) e
  o que está pendente (verbo, prefixo, unidade).
- **xah-fly-keys:** teclas posicionadas por frequência de uso real
  (não pela ordem alfabética do nome em inglês) e `Espaço` como tecla
  líder acessível com o polegar — os dois já eram verdade no nas-keys
  antes deste redesenho, e continuam.

## 9. Como alterar este sistema

1. **Mude primeiro este documento (`docs/keys.md`).** Se a mudança
   quebra uma regra da §3, pare e pense de novo — as regras existem
   pra manter o sistema previsível.
2. **Depois, `crates/nas-keys/keymap/default.toml`** — a tecla, o
   objeto ou o prefixo novo, e a descrição em `[commands]`. Um
   movimento horizontal ou vertical novo só precisa de uma linha em
   `[visual]`: o objeto correspondente é derivado dali (§5.3), não se
   declara separado.
3. Se a mudança precisar de lógica nova (não só de dados), ela vai em
   `crates/nas-keys/src/machine.rs` — mas só se for genérica (vale pra
   qualquer verbo/unidade/prefixo), nunca uma tecla específica
   hardcoded.
4. **Mantenha os testes passando** (`cargo test -p nas-keys`) e cubra
   o comportamento novo com um teste. Se o comportamento de uma tecla
   existente mudar, ajuste o teste correspondente e explique por quê
   no commit.
5. Se a mudança afeta `nas-term` (uma ação nova que precisa de um
   método do `Document`, por exemplo), ela só deveria mexer em
   `crates/nas-term/src/app.rs` — nunca direto em `nas-keys` nem em
   `nas-editor`, que não se conhecem.

## 10. Decisões em aberto

- **`Ctrl+b` (trocar de buffer) talvez pertença à classe alternar, não
  à classe sistema.** Trocar de buffer é "alternar entre duas coisas"
  — a mesma ideia central da classe Alt agora (§4). Hoje `b` vive em
  `[system]` porque trocar de buffer é o editor "como programa"; mas
  pela nova definição de Alt, a tecla poderia ser `Alt+b`, por
  exemplo. Não decidido: mudar isso tiraria `b` de `[system]`, exigiria
  escolher uma combinação Alt livre (as direções já são de janela) e
  provavelmente quebraria o hábito de quem já usa `Ctrl+b`. Ver §9
  antes de mexer nisso.
