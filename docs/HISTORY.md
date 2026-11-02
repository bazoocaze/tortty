# Histórico

Registro datado do que foi feito: narrativa do dia, não leitura corrente. O que
continua valendo é promovido para [`MIGRATION.md`](MIGRATION.md) (estado),
[`BENCHMARKS.md`](BENCHMARKS.md) (número) ou [`DEBUG.md`](DEBUG.md)
(armadilha). O histórico da fase Python fica no outro repositório.

Formato de cada linha: **data — o que — como foi verificado**.

## 2026-11-01 — fase 1: o experimento de viabilidade

Pergunta: reescrever em Rust resolve distribuição e desempenho? Decisão de
abortar era "se não fizer ≥3× o MB/s do Python e não abrir vim/mc, não paga".

- **Núcleo + app do zero** (`bzmux-core` sem GUI/pty/GPU, `bzmux` com
  `portable-pty` + `winit` + `wgpu` + `swash`). Verificado com 23 testes de
  núcleo, `vim`, `mc` e `dmesg` na tela.
- **Medição**: 8–9,5× o Python em fps e MB/s no doom-fire, 29–46× no desenho,
  `dmesg` de 1397 linhas em 2,1 ms contra 42,6 ms. Números e condição em
  [`BENCHMARKS.md`](BENCHMARKS.md).
- **Debug socket** com o mesmo protocolo JSON do Python, para o mesmo
  `benchrun.py` medir os dois.
- **Armadilha registrada**: `pkill -x bzmux` mata o `bzmux` do pipx (mesmo
  nome) e `pkill -f bzmux` mata o shell que chamou. Daí `tools/devrun.sh` +
  `tools/devkill.sh` por PID file com conferência de `/proc/<pid>/exe`.
- **Armadilha registrada**: `request_inner_size` é sugestão do WM; sem corrigir
  em passos de célula o benchmark mede 60x18 achando que mediu 61x18 (é o
  `want`/`adjust_left` em `app.rs`).

## 2026-11-01 — decisão de migrar, inventário e docs

- **Decisão**: migrar (o critério de 3× foi batido com folga; distribuição
  também). Rota única `winit`+`wgpu`, sem manter o stack GTK como alternativa.
- **Inventário de paridade**: 4 levantamentos independentes do código Python
  (núcleo/parser, render/cores/imagens, entrada/socket/infra, config/abas/CLI)
  viraram as 250 linhas de [`MIGRATION.md`](MIGRATION.md), cada uma com
  `arquivo:linha` do Python e estado aqui.
- **Docs reestruturados** no formato do projeto Python: `MIGRATION`, `BENCHMARKS`,
  `DEBUG`, `PORTABILITY`, `LIMITATIONS`, `BACKLOG`, `HISTORY` + `AGENTS.md`.

## 2026-11-01 — higiene: fmt, clippy, um comando

- `rustfmt.toml` e lints de workspace (`all`, `correctness`, `suspicious`,
  `pedantic`); 113 avisos zerados. `unsafe_code = deny` com `#[allow]` local no
  único bloco (`SO_PEERCRED`).
- `tools/check.sh` roda no pre-commit (`.githooks`) e no CI — mesmo script, para
  hook e CI não discordarem. Verificado: `./tools/check.sh` verde.
- **Bug**: o verbo `keys` do debug socket tinha uma segunda tabela de teclas
  (F5 saía com 1 byte, `alt+left` perdia o modificador). Agora chama o encoder
  real (`keys::encode_parts`). Verificado: `tab`=1 B, `shift+tab`=3 B, `f5`=5 B,
  `alt+left`=6 B.

## 2026-11-02 — SGR completo (C5, C6, C7)

- `Cell` ganhou `ul` (estilo 1–5) e `ul_color`; `flags` passou a ter itálico,
  faint, pisca, riscado e overline. `SGR 22` desliga negrito **e** faint, como
  o xterm; `21` é sublinhado duplo (é o que `bat`/`delta` querem).
- Desenho: itálico usa a face itálica do `fontdb`; negrito-itálico ganhou busca
  própria (`bold_italic`), porque a ordem antiga de candidatos fazia
  "negrito+itálico" sair em pé. Curvo/pontilhado/tracejado são feitos de
  retângulos — não há geometria curva no pipeline.
- Verificado na tela: 14 combinações de SGR numa sessão só (`negrito`,
  `faint`, `itálico`, riscado, overline, os 5 sublinhados, `58` em 256 e em
  truecolor, reverso) + 4 testes novos no núcleo (26 no total).

## 2026-11-02 — mouse reporting (F1-F6, F19, F20)

- `mouse.rs` porta o `mouse.py`: botão/modificador/movimento, SGR (`?1006`) e o
  protocolo antigo (release = botão 3, coordenada saturando em 222).
- `?1005` (coordenada UTF-8) implementado — o Python ignora; sem ele, coluna
  acima de 222 em protocolo antigo reporta errado por saturação.
- `app.rs` ganhou `on_cursor_moved`/`on_mouse_input`/`on_wheel` (o `window_event`
  passava de 100 linhas). Clique com mouse ligado volta ao vivo antes de
  reportar (F19); roda lendo histórico continua rolando a visão, não vai ao app.
- Verificado com sonda que lê o *recebido* (não o desenhado): clique 1 e 3 e
  roda em `?1000/?1006` → `ESC[<0;11;4M`, `ESC[<0;11;4m`, `ESC[<2;11;4M`,
  `ESC[<64;…M` + `m`. E com `vim -c 'set mouse=a'`: clique na linha 6 moveu o
  cursor para 7,7 (lido no `state` e na barra de status).

## 2026-11-02 — seleção, clipboard e paste (F8, F9, F12-F16, G14, D5-D7)

- `bzmux-core/select.rs`: âncora absoluta (`(linha, coluna, geração)`) e texto
  por `span`, como o Python — a seleção sobrevive a rolar e a output chegando,
  e morre quando o histórico é reescrito (`CSI 3J`/RIS/reflow), em vez de
  copiar texto errado.
- Desenho: inversão de verdade, inclusive para célula sem cor (o default
  resolve pelo outro extremo); a mesma regra vale para `SGR 7`.
- `clip.rs` com `arboard` sem a feature de imagem (binário segue 10 MB, sem
  `libX11`: o arboard fala o protocolo direto).
- Atalhos: Ctrl+Shift+C/V antes do encoder (senão Ctrl+Shift+C vira `^C`),
  botão do meio cola o PRIMARY, Shift+PgUp/PgDn/Home/End navegam o histórico,
  roda vale 3 linhas por entalhe e fração de trackpad acumula.
- **Armadilha medida**: coordenada do `xdotool` não é a do `winit` (moldura da
  janela). Teste de arrasto calibra o deslocamento movendo o mouse para dois
  pontos conhecidos e lendo `cursor_px`/`cell_at` do `state`.
- Verificado de ponta a ponta: arrasto em "CCC DDD" → inversão na tela, `state`
  com âncoras `a=(1,0) b=(1,6)`, Ctrl+Shift+V e botão do meio entregaram
  "CCC DDD" ao `cat` (conferido no arquivo e na tela).

## 2026-11-02 — F1 fechada nos itens de teclado, scroll e indicador

- **D8**: indicador `↑N` no canto superior direito (selo invertido, com glifo
  do atlas). Sem ele, rolar para trás é indistinguível de app parado.
- **D6/D7**: Shift+PgUp/PgDn/Home/End navegam; tecla de verdade volta ao vivo
  (modificador não, senão o Shift do Shift+PgUp já resetava antes de rolar).
- **E5**: Ctrl+Space agora é NUL; a tabela genérica produzia `CSI 1;5 `.
  Conferido byte a byte pelo verbo `keys`, que passou a devolver o hex.
- **E9**: dead keys já funcionam sem código nosso (o winit compõe e entrega
  texto); verificado com `xdotool key dead_tilde a` → `ãéù`.
- **Ferramenta**: `tools/devkill.sh` agora varre as sobras deste checkout (o
  PID file só guarda a última). O `cargo build` troca o arquivo, então o
  processo antigo aparece como `bzmux (deleted)` — é preciso tirar o sufixo
  antes de comparar, senão a sobra sobrevive e responde no lugar do build novo.

## 2026-11-02 — grade inicial: uma coluna perdida e o filho nascendo cedo

Sintoma: `bat`, `dmesg` e `seq` "embaralhados" nos primeiros KB. Não era
parser (a mesma saída lida de arquivo reproduzia igual): eram duas coisas.

- `round(cols*cw)` pedia pixels a menos (1304,4 → 1304 → 99 colunas). Passou a
  `ceil`.
- o filho nascia antes do tamanho final, então o primeiro resize do WM chegava
  com ele escrevendo. Agora o `apply_size` roda primeiro e o `spawn_shell` só
  acontece no primeiro tamanho aplicado.

Verificado: `--cols 100 --rows 10` → `stty size` = `10 100` e linha de 250
caracteres quebra em 100/100/50. (A suspeita inicial de "texto intercalado" era
leitura errada minha: as linhas 17 e 18 do `keys.rs` quebravam nos lugares
certos.)

## 2026-11-02 — instalador: `bzmux` é o Rust, `tortty-python` é o Python

O Python já foi renomeado no pipx (`~/.local/bin/tortty-python`), então o nome curto
ficou livre e o `install.sh` daqui passa a ocupá-lo.

O instalador é o oposto do Python em tamanho: lá são pacotes de distro
(`python3-gi`, `gir1.2-gtk-3.0`, cairo, pipx) e um passo do pipx; aqui é
**copiar um arquivo**. O que ele faz de fato é o que dá errado quando falta:
avisa se `bzmux` já existe no `PATH` (o shim do pipx não tem "pipx" no nome, o
alvo tem), instala com `install` em vez de symlink para o `target/` (senão
`cargo clean` deixa o comando quebrado), grava `.desktop` + ícone SVG, e
verifica `--version` + `ldd` sem "not found" antes de dizer que deu certo.

Detalhe que só apareceu depois: a primeira versão do instalador do Python
(`tortty-python/shortcut.py`) apagava `bzmux.desktop`/`bzmux.svg` como "nome antigo" —
e esses nomes passaram a ser do Rust, então rodar aquele instalador apagava a
entrada do menu daqui. Provado num `XDG_DATA_HOME` temporário (o `install()`
removia os dois arquivos), reportado e corrigido no `tortty-python`; revalidado aqui
com `--install-shortcut`/`--remove-shortcut` sobre os arquivos reais (md5 igual),
e o caso virou teste em `bzmux/checks.py`.

Dois bugs meus no caminho: a função do modo se chamava `install`, então o
`install -m 755` de dentro dela chamava a si mesma (recursão silenciosa); e o
teste de escrita usava `-w` num diretório que ainda não existia, o que pedia
`sudo` para instalar em `/tmp`. Funções renomeadas (`do_install`) e o teste
sobe até o ancestral existente.

## 2026-11-02 — F2: reflow no resize (A22, A23)

Antes: `resize` só esticava/cortava cada linha. Estreitar a janela **comia** o
texto da direita; alargar deixava o texto picado em colunas de 80. Agora o
resize do buffer principal junta as linhas "moles" (`wrapped`) em parágrafos e
re-quebra tudo na largura nova.

Decisões que valem registro:

- **corte** só de espaço em branco com fundo default (`Cell::is_blank`). Espaço
  com fundo pintado é conteúdo — barra de status do `vim`, `git diff --color`,
  `bat --style=full` são linhas de largura inteira pintada;
- **cursor** não vem de `cy`: vem do *offset dentro do parágrafo*, então 15
  células em 5 colunas terminam na 3ª linha, coluna 5, com quebra pendente
  (é o `wrap_pending` do VT, não uma coluna ocupada);
- **par largo** não é partido: se o corte cairia entre o caractere e sua
  continuação, a linha quebra uma coluna antes;
- `scrolled` continua monotônico e `abs_gen` sobe: a numeração absoluta muda,
  então a seleção morre (igual ao Python) em vez de copiar texto errado.

Provado com 6 testes novos (`reflow_*`, 38 no total) e no app: 30 parágrafos de
152 colunas, `100x20 → 50x20` re-quebra sem perder nada, `→ 120x20` junta de
volta na largura inteira.

## 2026-11-02 — F2: a visão não é arrastada pelo output (D10)

Quem rolava para o histórico e ficava lendo era empurrado: chegava uma linha e
a visão descia junto com o conteúdo. Agora `drain` guarda `scrolled` e o
tamanho do histórico antes do `feed` e pede a `Term::view_after_output` a
correção — `off + linhas_que_saíram - linhas_que_caíram_do_teto`.

A segunda parcela importa: com o histórico cheio (teto de 1000), cada linha
nova empurra `d` e descarta `d` das velhas — se o descarte não entrasse na
conta, a visão andaria para o vazio.

Verificado no app: `view_off` 27 → 29 → 31 com dois `echo` chegando, e a
primeira linha visível continuou "echo linha-12". Dois testes no núcleo
(inclusive com `scrollback_limit = 3`).

### F2 fechada

Um detalhe que faltava: `CSI 3J` esvaziava o histórico mas a `view_off`
continuava no valor antigo, então a tela mostrava linha que não existe (ou
linha errada). Agora `drain` prende `view_off` ao tamanho do histórico depois de
cada `feed`. Verificado com o app: `view_off` 29 → 0 e histórico vazio.

Fica registrado o desvio: o reflow mata a seleção em vez de remapear a âncora
(mesmo comportamento do Python). Remapear é a melhoria M-P, porque o `reflow`
já sabe o `(parágrafo, offset)` de cada linha velha.

## 2026-11-02 — `bzmux --selftest` (N6)

Um comando que responde "o emulador ainda está inteiro?" sem display, sem GPU e
sem pty: 14 checks no núcleo (`checks.rs`), cada um montando o cenário e
comparando com o esperado — parser/cursor, autowrap e `?7l`, SGR e ED, truecolor
e paleta do tema, DEC special graphics + DA1 + CPR, alt screen e DECSTBM,
histórico e ED3, reflow, seleção sobre histórico, bracketed paste, CSI com
intermediário (DECSCUSR e soft reset), config, OSC de cor e dano.

Entrou no `tools/check.sh` (que roda no pre-commit) e no CI. Três expectativas
minhas estavam erradas e o check pegou: o backspace depois de `z	w` sobrescreve
o `w`, o `DECSCUSR 4` guarda `(forma 1, pisca false)` e `ECH 2` deixa `"  c"`.
