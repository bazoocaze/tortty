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
| `state` | — | `ok` + `cols`, `rows`, `cursor` `[row,col]`, `title`, `view_off`, `scrolled`, `scrollback`, `alt`, `sync`, `cell` `[w,h]`, `font`, `font_px`, `tabs`, `workspaces`, `errors` |
| `text` | `where` (`view`\|`history`\|`screen`), `start`, `count` | `lines` (strings) |
| `write` | `data` (string) | `ok` (escreve no pty) |
| `keys` | `data` (spec tipo `ctrl+shift+t`, `f5`, `alt+left`) | `keys: [n]` bytes escritos no pty |
| `resize` | `cols`, `rows` | `requested` (o grid **pedido**; confira em `state`) |
| `prof` | `on` (bool, opcional) | contadores: `draw_ms_call`, `draw_calls`, `parse_*`, `pty_*` |
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
