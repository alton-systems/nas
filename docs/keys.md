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
  *onde* o radical age — texto, intensidade, espaço (janelas) ou
  sistema (o editor como programa). Ver §3.
- **Verbo**: uma tecla que inicia uma ação sobre uma região de texto
  (`d` apagar, `c` copiar, `x` recortar). Espera um **objeto**, a
  menos que já exista uma seleção ativa.
- **Objeto**: o que vem depois de um verbo, definindo a região sobre a
  qual ele age — outro radical de movimento, o próprio verbo repetido,
  ou "o arquivo inteiro". Ver §4.3.
- **Unidade**: uma tecla que, sozinha, espera uma direção para
  completar um movimento maior que um caractere ou uma linha — `w`
  (palavra) e `p` (parágrafo). Funciona tanto sozinha (movimento puro)
  quanto depois de um verbo (formando um objeto composto, ex. `d w ;`).
- **Composto**: a sequência completa de teclas até ela resolver numa
  ação — `d ;`, `d w ;`, `g l`, `SPC f` são todos compostos.
- **Prefixo**: uma tecla que, sozinha, não faz nada e espera mais uma
  tecla pra escolher entre um conjunto fechado de ações — `g` (ir
  para) e `Espaço` (líder).
- **Frase**: uma sequência de compostos digitada no estado Sequential,
  mostrada antes de executar — é o que a IA vai propor, no marco 4.
- **Estado**: Insert, Visual ou Sequential — ver §4.4.

## 3. As regras (o contrato do sistema)

1. **Um radical, um significado.** Se `k` é "para baixo", ele é "para
   baixo" sozinho, depois de um verbo, e em qualquer prefixo.
2. **O modificador define a classe.** A mesma tecla em outra classe
   mantém a ideia (a direção, por exemplo) e muda o "mundo" onde ela
   age: texto, janelas ou sistema.
3. **Shift intensifica.** Leva o significado ao extremo.
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
   cada uma faz, na barra de status.

## 4. As classes

| Classe | Como se aciona | Significado | Estados em que vale |
|---|---|---|---|
| Texto | tecla sem modificador | mover e editar o texto | Visual |
| Intensidade | Shift | o mesmo significado, no extremo | Visual |
| Espaço | Alt | janelas e painéis | todos |
| Sistema | Ctrl | o editor como programa: arquivos, buffers, sessão | todos |
| Ir para | prefixo `g` | saltar para um destino | Visual |
| Líder | prefixo `Espaço` | comandos de alto nível sobre o texto | Visual |

Para decidir onde uma ação nova entra: mexe no **texto**, sem
modificador; mexe na **organização da tela**, Alt; mexe no **editor
como programa**, Ctrl; é um **comando de alto nível**, líder.

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
| `p k` / `p l` | próximo parágrafo / parágrafo anterior |

`w` e `p` são radicais de unidade: sozinhos, não fazem nada e esperam
uma direção. Uma direção sem sentido para a unidade (ex.: `w k`) não
faz nada — a unidade é cancelada, como um prefixo.

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
| qualquer movimento **vertical** (`k`, `l`, `K`, `L`, `p k`, `p l`) | por linha: linhas inteiras, da linha do cursor até a linha de destino, inclusive. `d k` apaga a linha atual e a de baixo; `d K` apaga da linha atual até o fim do arquivo |
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
prefixo ou unidade pendente, em qualquer um dos dois casos.

### 5.4 Estados

| Tecla | Efeito |
|---|---|
| `Enter` (Visual) | entra no Insert |
| `Alt+Espaço` | alterna entre Insert e Visual. É uma **exceção conhecida** à classe Alt, mantida porque alternar o estado é a ação mais frequente e precisa ser acessível com uma tecla em qualquer estado |
| `Espaço Espaço` | entra no Sequential |
| `Enter` / `Esc` / `Backspace` (Sequential) | executa a frase / cancela / apaga a última tecla |

### 5.5 Prefixo `g` (ir para)

| Teclas | Destino |
|---|---|
| `g g` | início do arquivo |
| `g l` | uma linha (pede o número) |
| `g c` | uma posição em caracteres (reservado, não implementado) |
| `g f` | a definição de uma função (reservado, depende de LSP) |

### 5.6 Prefixo `Espaço` (líder)

| Teclas | Ação |
|---|---|
| `Espaço f` | buscar |
| `Espaço r` | substituir (reservado, não implementado — ver §7) |
| `Espaço h` | ajuda (reservado, não implementado — ver §7) |
| `Espaço Espaço` | entrar no Sequential |

Os movimentos por palavra e parágrafo **não** ficam no líder — são as
unidades `w` e `p` (§5.1).

### 5.7 Classe sistema (Ctrl, em todos os estados)

| Atalho | Ação |
|---|---|
| `Ctrl+s` | salvar |
| `Ctrl+q` | sair do editor (pergunta se houver alterações não salvas, em qualquer buffer) |
| `Ctrl+o` | abrir arquivo (num buffer novo, ou troca pro já aberto) |
| `Ctrl+b` | trocar de buffer |
| `Ctrl+k` | fechar o buffer atual (pergunta se tiver alterações não salvas; fechar o único buffer aberto deixa um vazio no lugar) |

`o` e `b` não são mais comandos do Visual — viraram `Ctrl+o` e
`Ctrl+b`, porque abrir arquivo e trocar de buffer são o editor agindo
como *programa*, não uma edição de texto.

Não use combinações de Ctrl que cheguem iguais a outras teclas no
terminal: `Ctrl+i` (Tab), `Ctrl+m` e `Ctrl+j` (Enter), `Ctrl+[` (Esc),
`Ctrl+h` (Backspace em alguns terminais).

### 5.8 Classe espaço (Alt, em todos os estados)

| Atalho | Ação |
|---|---|
| `Alt+j` / `Alt+;` / `Alt+k` / `Alt+l` | ir para a janela à esquerda / direita / abaixo / acima |

O nas ainda não tem janelas: nele, essa classe fica só documentada —
ver §7.

## 6. Teclas reservadas (não implementadas)

| Teclas | Reservadas para |
|---|---|
| `i` / `o` | objetos "dentro de" e "em volta de" (como o `i"` e o `a(` do Vim) |
| números `0`–`9` | contagem antes de um comando (ex.: `3 w ;`) |
| `m` | prefixo livre para atalhos do próprio usuário, vazio por padrão |
| `g c` | ir para uma posição em caracteres |
| `g f` | ir para a definição de uma função (LSP) |
| `Espaço r` | substituir (query-replace) |
| `Espaço h` | ajuda |
| `Alt+j`/`;`/`k`/`l` | gerência de janelas |

## 7. O que não está implementado, e por quê

- **`Espaço r` (substituir):** precisa de um fluxo interativo de
  confirmação por ocorrência (como `query-replace` do Emacs) — maior
  que uma linha de entrada simples. Fica pra quando esse fluxo
  existir.
- **`Espaço h` (ajuda):** não tem pra onde apontar ainda — não há um
  catálogo de comandos navegável no nas-term. As dicas de verbo/
  prefixo/unidade (regra 8) já cobrem boa parte da necessidade
  imediata.
- **Gerência de janelas (classe Espaço/Alt):** `nas-term` não tem
  múltiplos painéis — só um buffer visível por vez (trocar de buffer
  não divide a tela).
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
  universal — nosso `Esc` cancela verbo, prefixo ou unidade do mesmo
  jeito. `kill-line`/`kill-ring` também inspiram "apagar copia pra
  área de transferência".
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
   objeto ou o prefixo novo, e a descrição em `[commands]`.
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
