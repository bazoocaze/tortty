# Desenvolvimento: ferramentas, verificação e armadilhas

Manual operacional. **Ler antes de rodar qualquer ferramenta gráfica**: o
primeiro item abaixo já custou o terminal de trabalho de alguém.

## Nunca `pkill` — PID file, sempre

`pkill -f bzmux` mata o **shell que chamou** o comando, porque o padrão aparece
na linha de comando dele. `pkill -x bzmux` mata o `bzmux` do **pipx**, que tem
exatamente o mesmo nome do binário de dev deste repositório — foi assim que um
`pkill` aqui derrubou a sessão do terminal do usuário.

As ferramentas só matam PID registrado, conferindo `/proc/<pid>/exe` antes:

```sh
./tools/devrun.sh --cmd 'mc' --cols 100 --rows 30 --debug-socket /tmp/bz.sock
./tools/devkill.sh            # mata só o PID do arquivo /tmp/bzmux-rs.pid
./tools/devkill.sh --status   # mostra vida/exe sem matar
```

O `devrun.sh` recusa subir se já existe processo vivo com aquele PID file (evita
duas instâncias disputando o mesmo socket).

## Sondas (o que dá para ligar sem recompilar)

| sonda | efeito |
|---|---|
| `--cmd 'CMD'` | roda `sh -c CMD` em vez de `$SHELL` |
| `--cols N --rows N` | tamanho inicial em células (o WM pode ajustar) |
| `--font-family`, `--font-size PT`, `--dpi N` | fonte em pontos; `--dpi` é o `Xft.dpi` |
| `--scrollback N` | teto de linhas do histórico |
| `--no-vsync` | desenha o mais rápido possível (medir vazão) |
| `--prof` | liga contadores de `draw`/`parse`/`pty` (consultados por `prof`) |
| `--debug-socket CAMINHO\|auto` | socket JSON de introspecção (`auto` = `$XDG_RUNTIME_DIR/bzmux-<pid>.sock`) |
| `--debug-log ARQ` | log de eventos (`logln!`) |
| `WGPU_BACKEND=gl\|vulkan` | força backend do `wgpu` (válvula para GPU problemática) |
| `WGPU_ADAPTER_NAME=…` | escolhe adaptador (útil para testar llvmpipe) |
| `SHELL=…` | shell padrão quando não há `--cmd` |

## Verificação visual sem roubar a tela do dono

Abrir a janela na sessão gráfica de quem está trabalhando atrapalha: ela sobe por
cima e, com `xdotool`, rouba o foco. A verificação visual daqui roda num display
próprio — a janela existe, mas não na tela de ninguém:

```sh
./tools/visual.sh            # Xvfb + tema fixo + captura + comparação; sai 0/1
./tools/visual.sh --keep     # guarda as capturas em /tmp/bzmux-visual
```

O `tools/visual.sh` é o dono dessa receita (display, `HOME` de mentira com
`bzmux.conf` de tema fixo, espera do primeiro frame, captura pelo `state`/`xid`, e
as quatro checagens de pixel via [`tools/pixel.py`](../tools/pixel.py)). Para
sondar à mão uma captura: `tools/pixel.py shot.png 100,50` (cor de um ponto),
`--scan X`/`--row Y` (amostra por célula) e `--diff a.png b.png` (quantos pixels
mudaram e onde).

Detalhes que economizam tempo:

- **O `xid` vem do `state`**, não de `xdotool search --class bzmux`: num display
  com mais de uma instância a busca pega a janela errada e a captura sai vazia.
- **`HOME` de mentira** é o que instala fonte de teste ou configuração sem tocar
  no `~/.config` real; PID file separado evita que o `devkill` derrube a
  instância que o dono esteja usando.
- **`import -window`/`xwd` não precisam de foco** — é `xdotool` que precisa, e no
  Xvfb ele manda para a janela do display isolado.
- **Sem `/dev/dri` o `wgpu` cai em llvmpipe, e llvmpipe desenha** sob Xvfb (foi
  assim que emoji, ligadura e dano foram conferidos em 2026-11-02). Serve para
  conteúdo e geometria; número de desempenho só vale na GPU.

## Verificação com janela (sem depender de olho)

```sh
./tools/devrun.sh --cmd 'mc' --cols 90 --rows 24 --debug-socket /tmp/bz.sock
PID=$(cat /tmp/bzmux-rs.pid)
WID=$(wmctrl -lp | awk -v p=$PID '$3==p {print $1; exit}')
xwd -id "$WID" -out /tmp/x.xwd && convert /tmp/x.xwd /tmp/x.png   # olhar
xdotool windowactivate "$WID"; xdotool mousemove --window "$WID" 200 200
for i in $(seq 6); do xdotool click 4; done                        # roda do mouse
```

Depois, conferir **por estado**, não por pixel: `state.scrollback`,
`state.view_off` e `text where=view` dizem se a rolagem aconteceu. Pixel diz que
algo foi desenhado; estado diz o quê.

Para teclado, o caminho é o real (`xdotool key` → XTest → winit → encoder →
pty), não `keys` do socket: `keys` testa o encoder, `xdotool` testa a fronteira.

## Socket de introspecção

Unix socket, uma requisição JSON por linha, uma resposta JSON por linha. `ok:
true` ou `ok: false` com `err`. O mesmo protocolo do bzmux Python, de propósito:
o `tools/benchrun.py` roda nos dois.

| verbo | entrada | resposta |
|---|---|---|
| `state` | — | `ok` + `cols`, `rows`, `cursor` `[row,col]`, `title`, `view_off`, `scrolled`, `scrollback`, `alt`, `sync`, `cell` `[w,h]`, `font`, `font_px`, `tabs`, `workspaces`, `errors`, `draws`, **`drawn`**, **`skips`**, **`last_frame`**, `last_damage`, `frames_all`, `rows_max`, `all_reason`, `xid` |
| `text` | `where` (`view`\|`history`\|`screen`), `start`, `count` | `lines` (strings) |
| `write` | `data` (string) | `ok` (escreve no pty) |
| `keys` | `data` (spec tipo `ctrl+shift+t`, `f5`, `alt+left`) | `keys: [n]` bytes escritos no pty |
| `resize` | `cols`, `rows` | `requested` (o grid **pedido**; confira em `state`) |
| `prof` | `on` (bool, opcional) | contadores: `draw_ms_call`, `draw_calls`, `parse_*`, `pty_*` |
| `osc` | `tab` (opcional, padrão a ativa) | os últimos 8 OSC que o parser viu, do mais novo para o mais velho: `hex`, `text` (lossy), `truncated`, `title` (o resultante). É a sonda de "por que o título da aba está com lixo" |
| `log` | `data` | escreve no log de debug |
| `quit` | — | fecha a janela |

`state` é a fonte da verdade para o harness: a resposta tem a grade **depois** do
WM, que é o que importa. `resize` devolve o pedido, não o resultado — quem confere
é o cliente, com `state`, e é por isso que o `benchrun.py` insiste até bater.

## Armadilhas

### winit

- **`request_inner_size` é sugestão.** Pedir `cols * cw` pixels pode voltar com
  1 célula a menos (o WM arredonda e o `floor` corta). O app guarda o grid pedido
  (`want`) e corrige em passos de célula até bater, até 4 tentativas; sem isso o
  benchmark mediria 60x18 achando que mediu 61x18.
- **`RedrawRequested` não é "a cada frame"**: é sob demanda. Em `--no-vsync`
  quem puxa o desenho é o `request_redraw` depois de cada leitura de pty; em uso
  normal é o vsync.
- **`Resized` chega com tamanho físico**; a grade tem de ser recalculada com o
  `cw`/`ch` atuais, e o `surface` do `wgpu` reconfigurado no mesmo evento.
- **Teclado**: o `logical_key` importa para texto; `physical_key` para
  modificadores e teclas de função. Usar só um dos dois erra acento ou atalho.

### wgpu

- **`InstanceDescriptor`, `RequestAdapterOptions` e `DeviceDescriptor` mudaram
  de forma** entre versões: campos obrigatórios novos, sem default. Compilar
  contra a versão do `Cargo.lock`, não contra a documentação da web.
- **O `surface` precisa ser reconfigurado a cada resize** (formato, tamanho),
  senão frame com lixo ou `Validation Error` no log.
- **`Usage::COPY_DST` no atlas** e `write_texture` no mesmo comando que desenha
  exige cuidado: subir o atlas *antes* do render pass do frame.
- **Formato do atlas é `R8`** (alpha): o shader coloriza pela cor da instância.
  Usar `Rgba8` quadruplica a memória e não melhora nada.

### fonte (fontdb + swash)

- **Bitmap de cor rouba glifo da primária**: `glyph_bitmap` existe para o texto
  fora da grade (barra, sidebar, selo, toast) e resolve cor → máscara. Se a
  passada de cor varrer a cadeia de socorro inteira antes de qualquer máscara,
  uma fonte de emoji que mapeie ASCII vence: a **Noto Color Emoji** mapeia
  dígito e devolve bitmap de 19x18 px, então o número da aba virava um
  retângulo de duas células. A ordem certa é a fonte da célula primeiro (cor e
  máscara na mesma face) e só depois a cadeia; a grade não sofre disso porque
  escolhe a face pelo cmap (`face_of`). Teste: `digito_vem_da_primaria_em_mascara`.
- **Não há fontconfig**: a escolha é lista de preferência fixa + varredura do
  sistema. Máquina sem monoespaçada → o app sai com mensagem, não com `.notdef`.
- **pt→px**: `px = pt * dpi / 72 * scale_factor`. É o que faz a célula bater com
  a do Pango/GDK (`Xft.dpi`); errar isso desalinha a grade em 8%.
- **Métricas** vêm do glifo `M`, não do `font_extents`: é o que o terminal
  precisa (largura de avanço), não a caixa tipográfica.

### pty

- **`CommandBuilder` não herda o `cwd`** por conta própria: sem `cwd()`, o
  `mc` abre em `$HOME` mesmo que o app tenha sido chamado de outro diretório.
- **`SHELL` exportado** é o que roda de fato (`--cmd` troca o comando, não o
  `SHELL`): o filho herda o ambiente inteiro e passa a mentir sobre si mesmo.
- **`TERM=xterm-256color`** e **`COLORTERM=truecolor`** precisam ser exportados;
  sem eles `mc` desenha em 8 cores e `vim` não liga o truecolor.

### parser / grade (o que o Python já tinha aprendido)

- **Quebra pendente**: depois de escrever na última coluna, o próximo caractere
  quebra a linha — e um `\r\n` logo em seguida **não** pode contar como segunda
  quebra. Padrão de TUI que desenha até a borda depende disso.
- **Dano é a única fonte de repintura.** Caminho novo que escreve célula sem
  marcar `dirty[y]` não aparece na tela até algo rolar, e o bug parece de parse.
- **Wide char na borda direita**: não cabe, então a célula fica vazia e o glifo
  vai para a linha seguinte. Escrever no meio de um par largo (ou apagar metade)
  tem de limpar as duas células, senão sobra fantasma.
- **Âncora de histórico é distância do vivo, não linha absoluta**: `CSI 3J`
  apaga o histórico e uma âncora absoluta faz a visão saltar para o topo.
- **`CSI 3J`/`RIS`/troca de alt-screen** invalidam cursor, margens e charset:
  limpar só a grade deixa estado sujo entre aplicações.
- **`feed` com UTF-8 parcial**: o buffer pode terminar no meio de uma sequência;
  guardar os bytes e continuar na próxima chamada (o `dmesg` cortado no meio de
  um acento é o caso de teste).
- **`ST` de 8 bits**: `0x9c` fecha OSC/DCS/APC/PM/SOS como o `ESC \` (muito
  cliente manda isso em vez do ST de 7 bits). Só vale quando **não** é byte de
  continuação de UTF-8 — `М` é `d0 9c`, e cortar ali perde o resto do título.
  Um `ESC` que não é ST (dentro do corpo) **encerra** a sequência: empurrá-lo
  de volta para o payload colava o `ESC ]` do título seguinte dentro do título
  e a barra de abas mostrava lixo antes do texto (B4, 2026-11-04). A sonda é o
  verbo `osc` do socket, que devolve os payloads crus.
- **Texto de cromo não é sequência de terminal**: a regra é uma só,
  `bzmux_core::term::sanitize_title` (fora controle, zero-width, marcas de
  direção e BOM), e ela vale em **dois níveis**:
  - *desenho*: `Renderer::chrome_text` é o único lugar que desenha texto fora da
    grade (barra de abas, cartão da sidebar, selo de histórico, toast) e sanitiza
    ali — elemento de cromo novo herda a garantia sem chamar nada;
  - *produtores*: payload do OSC (`osc_dispatch`), basename do cwd
    (`Tab::cwd_name`) e nome de workspace (digitação do rename e o `Enter` que o
    confirma) saem limpos, porque além da tela eles vão para o título da janela,
    para o `state` do socket e para o cabeçalho do `dump` — um `\n` num nome de
    diretório quebra o formato do dump.

## Qualidade (linter e formatação)

```sh
./tools/check.sh          # a régua completa: fmt, clippy, teste, build, parsebench
./tools/check.sh --fix    # conserta o que é mecânico (fmt + clippy --fix)
```

O `pre-commit` roda `--fast` sozinho (instalado com
`git config core.hooksPath .githooks`); o CI roda o script inteiro. Se
`clippy` reclamar de algo que o código **quer** fazer, o `#[allow]` vai na
função com o motivo — não no topo do crate, e não em `#![allow]` global.

Antes de rodar GUI, lembre do resto deste arquivo: matar processo é por PID
file, e `pkill` por nome derruba sessão.

### Mouse

`mouse.rs` é puro (como o `mouse.py`): traduz `(botão, col, row)` em bytes do
protocolo. Quem decide se o evento vai para o filho é o `Term`
(`mouse_mode` 0/1/2/3, `mouse_sgr`, `mouse_utf8`). O `state` do socket expõe os
três, que é como se descobre, sem chutar, se o app ligou o mouse.

Sonda que fecha o ciclo (o que o app *recebeu*, não o que ele fez):

```sh
./tools/devrun.sh --cmd 'python3 /tmp/mtest.py' --cols 60 --rows 12 --debug-socket /tmp/bz.sock
# mtest.py faz tty.setraw(0), liga ?1000/?1006 e imprime o hex do que chegar
xdotool mousemove $((X+120)) $((Y+60)) click 1
# ler a tela: {"cmd":"text","where":"view"} -> 1b5b3c303b31313b344d = ESC[<0;11;4M
```

### Cromo: a linha 1 e a coluna da esquerda

Duas faixas da janela não são grade, e as duas moram no mesmo espaço do
`origin` do shader (`pad + bar`, `pad + side`), o que é a fonte de quase todo
erro de coordenada:

| faixa | desenho | teste de clique |
|---|---|---|
| barra de abas (linha 1, à direita da sidebar) | `y0 = -bar`, `x0 = 0` | `tab_at`: `py` em `[pad, pad+bar)`, `x` a partir de `pad+side` |
| sidebar (coluna da esquerda, desde a linha 1) | `y0 = -bar` no 1º cartão | `card_at`: `px < pad+side`, `i = (py - pad) / (2,4*ch)` |

A barra **some** com uma aba só (`bar_h = 0`), então tudo que depende dela tem
que perguntar `bar_h(ch)` na hora — e a grade ganha a linha de volta quando ela
some. O cartão tem 2,4 linhas de altura: para clicar no segundo, o `y` é ~1,5
linha abaixo do primeiro, não meia linha.

Correção de origem, quando o clique cai na faixa errada: mover o mouse e ler
`cursor_px`/`cell_at` do `state` (é exato, sem offset de moldura) e derivar
`side = px - col*cw`.

### Tamanho inicial da grade

Duas armadilhas juntas, e as duas aparecem como "texto embaralhado no
primeiro segundo":

1. Pedir a janela em `round(cols*cw)` dá pixels **a menos** que o grid precisa
   (100 células de 13,044 px = 1304,4 → 1304 → 99 colunas). Por isso o pedido é
   `ceil`.
2. O filho nascia antes de saber o tamanho final; o primeiro `resize` do WM
   chegava com o filho já escrevendo, e o que estava na tela ficava com a
   largura velha. Agora o `apply_size` roda primeiro e o `spawn_shell` só
   acontece no primeiro tamanho aplicado.

Sonda: `bzmux --cmd 'stty size'` tem que imprimir exatamente
`<rows> <cols>` do pedido, sem resize depois.

### Teclado

O verbo `keys` do socket passa pelo **encoder real** e devolve os bytes em hex,
que é como se confere `Ctrl+Space` (NUL), `Alt+←` (`CSI 1;3D`) e `F5`
(`CSI 15~`) sem adivinhação:

```sh
python3 -c 'import socket,json; s=socket.socket(socket.AF_UNIX); s.connect("/tmp/bz.sock");
s.sendall(b"{\"cmd\":\"keys\",\"data\":\"alt+left\"}\n"); print(s.recv(999))'
```

Cuidado ao testar: `ctrl+c`/`ctrl+\\` matam o filho (`SIGINT`/`SIGQUIT`), o app
fecha junto e o cliente fica lendo de um socket morto. Use um filho que ignora
os sinais (`sh -c 'trap "" INT QUIT; sleep 200'`).

Dead keys não têm código nosso: o `winit` entrega `Key::Dead` (que não gera
byte) e depois o texto composto em `event.text`. Teste com
`xdotool key dead_tilde a` e leia a tela pelo `text`.

## Medição

Ver [`BENCHMARKS.md`](BENCHMARKS.md) — é o único lugar com número. Resumo do
fluxo: `parsebench` para o parser, `tools/benchrun.py` para o app,
`tools/bench.sh` para a escada.

## Armadilha: erro X pendente mata o app no `set_title`

Sintoma, no X11:

```
thread 'main' panicked at winit-0.30/src/platform_impl/linux/x11/window.rs:1028
Failed to set window title: XError { description: "BadDrawable (invalid Pixmap or
Window parameter)", error_code: 9, request_code: 149, minor_code: 4 }
```

O `winit` trata **qualquer** erro X recebido durante um `flush` como fatal e
atribui o erro à chamada que estava sendo feita. Quando o WSI do Mesa (llvmpipe,
sem `/dev/dri`) cria e destrói pixmaps na nossa conexão, sobra um erro pendente e
a próxima `ChangeProperty` (título) leva a culpa — a janela morre mesmo estando
sadia, e o erro não é dela.

Como confirmar: `RUST_BACKTRACE=1` mostra `sync_title` na pilha, e o app **roda
normalmente** com a sonda abaixo.

Sonda: `BZMUX_NO_TITLE=1 bzmux` (não toca no título da janela). É sonda de
depuração, não configuração — só existe para conseguir usar o terminal enquanto
se investiga.

## Armadilha: GL sem display handle, e o `alpha_mode`

Duas coisas que só aparecem no backend GL:

- `InstanceDescriptor::new_without_display_handle_from_env()` faz o EGL
  responder `gl not compatible with provided surface`. O display handle não é
  opcional para o EGL; a instância é criada com
  `new_with_display_handle_from_env(Box::new(window))`.
- `alpha_mode: caps.alpha_modes[0]` pegava `PreMultiplied` no GL e a janela
  ficava translúcida. Terminal é opaco: `CompositeAlphaMode::Opaque` explícito.

O backend e a placa escolhidos aparecem na primeira linha do log:

```
bzmux: GPU: Vulkan llvmpipe (LLVM 20.1.2, 256 bits) (driver llvmpipe)
```

## Sandbox degradado: janela que não pinta nada

**Antes de investigar desenho, confira `/dev/dri` com `ls -la /dev/dri`.** Sem
os nós de device o `wgpu` fica só com llvmpipe. Sob um display próprio (Xvfb)
llvmpipe **desenha** — `tools/visual.sh` passa sem `/dev/dri`. Na sessão real do
dono, em 2026-11-02, o container sem `/dev/dri` fez o pty e o socket
funcionarem com a janela **sem receber frame**.

O primeiro campo a olhar é o par que separa as duas causas:

| `drawn` | `skips` | `last_frame` | leitura |
|---|---|---|---|
| > 0 | 0 | `drawn` | desenhou e apresentou |
| 0 | > 0 | `skipped (timeout/occluded)` | o desenho roda, a **superfície** recusa: ambiente (compositor/WM/sandbox), não o app |
| 0 | 0 | `nenhum` | o app nem chegou a desenhar: veja o log (`bzmux: GPU:`), pty e config |

**`drawn > 0` não prova que a tela desenhou.** Medido em 2026-11-03 no `sandbox` sem
GPU: com `WGPU_BACKEND=gl` o contador subia (`drawn 5`, `skips 0`) e o pixmap da
própria janela **não mudava um byte** — present que retorna sucesso e não pinta.
O teste que não mente é comparar a janela com ela mesma:

```sh
xwd -id <xid> -silent -out /tmp/a.xwd      # xid vem do `state` do socket
# ... manda o app pintar algo (verbo `write`) ...
xwd -id <xid> -silent -out /tmp/b.xwd
cmp -s /tmp/a.xwd /tmp/b.xwd && echo "não pinta" || echo "pinta"
```

Quando a superfície recusa o frame, o app escreve **uma vez** no stderr:

```
bzmux: a superfície não apresenta (timeout/occluded) — o app desenha, mas o
frame não chega à tela. Ambiente confinado sem GPU visível costuma ser isso:
confira /dev/dri e, no sandbox, use `sandbox @gpu`; ver docs/DEBUG.md
```

Sinais no log, nessa ordem, quando é o sandbox:

```
_amdgpu_device_initialize: amdgpu_get_auth (1) failed (-1)
libEGL warning: egl: failed to create dri2 screen
MESA: error: Failed to attach to x11 shm
```

Consequência prática: **verificação visual não é confiável nesse estado**. Nesse
caso o gate é outro: `tools/visual.sh` num Xvfb (prova pixel sem depender do
display do dono), mais o que não depende de pixel — `state` (grade, cursor,
imagens, `drawn`/`skips`/`rows_max`/`last_damage`), `--selftest`, `cargo test` e
`parsebench`.

No mesmo sandbox, o **cache do `cargo` já foi somente leitura**
(`/home/user/.cargo/registry`): dependência nova falhava com `Read-only file
system (os error 30)`. Hoje o perfil `@dev` do sandbox libera a escrita nesse
diretório (`sandbox @dev -- cargo build`), e sem ele o contorno é
`CARGO_HOME=/tmp/cargohome` (cópia do `~/.cargo` em tmpfs).

### Os três terminais dentro do `sandbox` (medido em 2026-11-02)

Mesmo sandbox, sem `/dev/dri`, tudo em llvmpipe. A diferença entre eles é
política de erro, não capacidade do ambiente:

| terminal | o que acontece | como se soube |
|---|---|---|
| `kitty 0.48.2` | **abre e desenha**; só reclama do DBus | `xdotool` acha a janela; o log só tem `glfw error 65544` (DBus) |
| `ghostty 1.3.1` | abre a janela, mas o GL degrada: `MESA: error: Failed to attach to x11 shm` em série | não sai; a janela existe e o log repete o erro de SHM |
| `bzmux` (Vulkan/lavapipe), até 1.0.4 | **abortava** (134) no `set_window_title` | panic da winit em `x11/window.rs:1028` |
| `bzmux` desde 1.0.5 | **não aborta**: o erro X que não é nosso é registrado e o app segue (hook abaixo) | `sandbox bzmux` vivo por 12 s, respondendo no socket |
| `bzmux` com `WGPU_BACKEND=gl` | também não aborta, mas **não pinta** (present no-op silencioso) | `xwd` da janela idêntico antes/depois |
| `sandbox @gpu bzmux` | **pinta de verdade** (`RADV RENOIR`, pixmap muda) | `xwd` antes/depois diferente |

Ou seja: sem `/dev/dri` este X server não apresenta superfície de GPU — nem por
Vulkan nem por GL. A saída é o perfil `@gpu` do sandbox, não um backend
diferente; o app avisa isso em uma linha no stderr.

### Erro X do WSI não pode matar o app

O `winit` guarda o último erro X da conexão e o `expect()`a na próxima chamada
que faz flush (`x11/window.rs:1028`, em `set_window_title`). Erro que **não é
nosso** — o WSI do Mesa deixa pendente ao criar/destruir pixmap — matava o
processo. `crates/bzmux/src/xguard.rs` registra o hook público do winit
(`platform::x11::register_xlib_error_hook`) e engole, registrando no stderr:

| código | onde | por quê |
|---|---|---|
| `BadDrawable` (9), `BadWindow` (3), `BadMatch` (8) | qualquer requisição | desenho: o WSI mexe em pixmap/janela, inclusive por requisição do núcleo |
| `BadAccess` (10) | só extensão (`request_code >= 128`) | é o `DRI3Open` negado no sandbox — **era este o que matava** |

Com `BZMUX_XERR=1` o hook imprime **todo** erro X que vê, com código,
requisição e se engoliu — foi assim que o `BadAccess` na `DRI3` apareceu:

```
XERR codigo=10 requisicao=130 menor=1 recurso=0x4000002 engolir=false
```

`BZMUX_NO_TITLE=1` continua existindo como sonda (não mexe no título), mas desde
o hook não é mais necessária para o app sobreviver.

### Captura de tela não é neutra

- **Janela obscurecida**: `xwd`/`import` de uma janela coberta devolvem o que
  está **na tela** naquelas coordenadas — o conteúdo de outra janela. Se a
  captura "não bate com o app", primeiro pergunte se ele está visível.
- **Grab de ponteiro**: `import`/`xwd` pegam o ponteiro e mostram crosshair (o
  "modo mira"); se o processo morre no meio, o ponteiro pode ficar preso até o
  cliente X sumir.
- Consequência: captura só em display virtual próprio (`tools/visual.sh`);
  nunca no display de quem está trabalhando.

