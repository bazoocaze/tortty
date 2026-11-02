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

## 2026-11-02 — release por tag (M8)

`.github/workflows/release.yml` (tag `v*`) builda `x86_64` e `aarch64`, e o
empacotamento é o `tools/release.sh` — o mesmo comando do local, não um caminho
paralelo de CI. Ele:

- confere `--check-version vX.Y.Z` contra o `Cargo.toml` (tag apontando para
  outro código é o erro mais fácil de cometer e o mais chato de descobrir);
- roda `bzmux --version` **no binário empacotado** — no cross, via
  `qemu-user-static`; sem qemu, avisa no log em vez de fingir que verificou;
- monta `bzmux-<versão>-<triple>.tar.gz` com binário + LICENSE + README e
  imprime o `sha256`, que é o formato que o `install.sh --from-release` espera.

Testado local: pacote de 3,9 MB, extrai, roda e responde `bzmux 0.1.0`;
tag errada (`v0.2.0` com `Cargo.toml` em `0.1.0`) sai com código 1.

Falta para fechar M8 de verdade: `windows-latest` e `macos-14`/`macos-13` na
matriz — dependem de M6 (ConPTY) e M7 (Metal), que estão em `PORTABILITY.md`.
Licença MIT entrou junto (o pacote leva o arquivo).

## 2026-11-02 — pisca do cursor e toast (G12, I12, K13)

Duas coisas que faltavam para o terminal "não parecer quebrado":

**Pisca (G12).** A fase não é estado acumulado: sai de `blink_epoch`
(`Instant`) dividido por 500 ms, então não existe timer que possa atrasar ou
desencontrar. `cursor_blink on` pisca, uma tecla ou colagem **reacende** na hora
(cursor que some no instante em que você olha é pior que cursor fixo) e
`CSI 6 q` com forma par (4/6) pede cursor fixo e vence. O `SGR 5` do texto usa a
mesma fase — o atributo existia desde a F1 mas não animava.

O loop só acorda no meio do período quando **há** algo piscando
(`pisca_ativo`), e o "tem célula com SGR 5?" é recalculado no `feed`, não em
cada tique. Verificado por A/B de capturas: com `on`, 432 px alternam entre
frames a 260 ms; com `off` (recarregado a quente), as capturas são idênticas.

**Toast (I12/K13).** O arquivo de config é editado num editor — que não mostra
o nosso log. Agora o erro aparece na tela: `config mantida — linha 3:
'afffont_size': chave desconhecida (+1)`, faixa invertida na primeira linha
(embaixo está o prompt, e o aviso é do terminal, não do programa), 3,5 s e
some sozinho. Mensagem igual seguida vira `(×N)` em vez de fila. Salvar certo
depois de um erro mostra `config recarregada`.

Dois detalhes que só apareceram testando: o primeiro tique reparseava o arquivo
e o erro saía duas vezes (agora o `mtime` do arranque já vale) e a condição do
"recarregada" olhava `config:` com dois-pontos, que a mensagem nova não tinha.

Benchmark depois de tudo: 950–985 fps / 40,8–42,3 MB/s a 61x18 — dentro da
faixa documentada, sem regressão.

## 2026-11-02 — abas (F4a: I1-I4, I13, I15-I17, I20)

O Rust deixou de ser "um terminal" e passou a ser o bzmux: `Tab` guarda
`term` + `pty` + o que é de olhar (visão, seleção, arrasto), e o `App` tem
`tabs: Vec<Tab>` com uma ativa.

O que veio junto:

| item | decisão |
|---|---|
| drenar **todas** as abas | canal não é ilimitado: um `yes` numa aba escondida encheria a memória. O que fica reservado à ativa é só o pisca e a visão |
| cwd da aba nova | `/proc/<pid>/cwd` do **descendente mais fundo** (via `task/<pid>/children`), não do shell: quem manda é o `vim`/`mc` que está na frente. Verificado: `cd /tmp` → aba nova nasce em `/tmp` |
| título | OSC 0/2 → `basename(cwd)` → `bzmux`; a janela segue a aba ativa (`rust_term — bzmux`), só quando muda |
| barra de abas | uma linha acima da grade (o `origin` do shader é `pad + bar`), e a grade de **todas** as abas encolhe uma linha; com uma aba só não existe barra |
| BEL | o rótulo da aba fica com `color3` por 800 ms — inclusive em aba de segundo plano, que é o ponto |
| dígito do atalho | tecla **física** (`Digit1..0`): `Ctrl+Shift+1` chega como `!` e no ABNT2 cada dígito shiftado é outro símbolo |
| `--cmd` | só vale para a primeira aba: repetir o comando de depuração na segunda não abriria shell |

Atalhos: `Ctrl+Shift+T` (nova), `Ctrl+Shift+W` (fecha), `Alt+1..9` (`0` = 10ª).
A aba cujo shell morre fecha sozinha; fechar a última fecha o app.

Verificado por socket (`tab op=new/select/close`, drenagem independente por aba,
`rows` de 10 → 9 com a barra) e por XTest (`ctrl+shift+t`, `alt+1`, `alt+3`,
`ctrl+shift+w` conferidos por `state`), mais capturas para o flash de BEL e a
inversão da aba ativa. Benchmark com uma aba: 993 fps / 42,7 MB/s — sem
regressão.

Falta para fechar F4: workspaces e sidebar (F4b: I5-I11), arrastar para
reordenar (I10) e roda na barra (I11).

## 2026-11-02 — F4 fechada: workspaces e o resto do cromo (I5-I11, I15, I20)

**Workspaces**: `parked` guarda os outros conjuntos inteiros (pty vivo, grade
própria) e trocar é swap de conteúdo — não há cópia de termo nem processo
morto. A ordem lógica é remontada na troca: trocar `parked[slot]` no lugar
embaralhava os cartões quando o corrente ia para uma posição mais alta (visto
no `state`, com os nomes trocados de lugar). A sidebar aparece com 2+
workspaces e come 18 colunas da grade; a barra de abas continua de ponta a
ponta, por cima dela.

Atalhos: Ctrl+Shift+T/W (aba), N/Q (workspace), Ctrl+Shift+F2 (config no
editor, em aba nova — P33), Ctrl+Shift+1..9, Alt+1..9 e Super+N/Q/1..9. Clique
esquerdo no cartão troca, **meio fecha** (no Python só o cartão tinha isso),
duplo clique renomeia no lugar (`|` é o cursor, 1ª tecla substitui o nome
antigo como o `Gtk.Entry` selecionado, Enter grava, Esc descarta).

Três armadilhas só apareceram testando:

| sintoma | causa | correção |
|---|---|---|
| barra de abas na linha 2 | desenhada em `y=0` com a origem já em `pad+bar` | desenha em `y=-bar`, a faixa reservada |
| clique na barra errava a aba | `tab_at` descontava a sidebar, que empurra a **grade**, não a barra | a barra começa na borda da janela |
| roda na barra não trocava nada | o X11 manda **dois** eventos por entalhe (press e release) com o mesmo sentido, e o segundo desfazia a troca | par deduplicado em 10 ms |

Verificação: `state` com `tabs/ws/ws_names` conferido a cada passo (ordem dos
workspaces estável ao trocar e ao fechar), captura da sidebar com e sem barra,
e a roda/arrasto conferidos com o mouse de verdade.

## 2026-11-02 — sidebar e barra de abas dividem a linha 1

A sidebar começava na linha 2 quando a barra de abas aparecia: a barra ia de
ponta a ponta e a coluna da esquerda ficava com um buraco na primeira linha.
Agora a barra começa onde a grade começa (`x0 = 0`, à direita da sidebar) e o
primeiro cartão começa em `y = -bar`, na linha 1, ao lado dela. `card_at` foi
junto: o topo do cartão é `pad`, não `pad + bar`. A ordem das consultas importa
— `card_at` antes de `tab_at`, senão o clique no cartão da linha 1 virava
clique na aba.

## 2026-11-02 — o backlog ganhou o que não é migração

Os dois documentos faziam o mesmo trabalho pela metade: o `MIGRATION.md` tinha
uma seção "melhorias propostas" e o `BACKLOG.md` dizia que a fila era o
`MIGRATION.md`. Agora cada um tem um assunto só — `MIGRATION.md` responde
"quanto falta para o Rust fazer o que o Python faz"; `BACKLOG.md` guarda o que a
migração não pede, com um **gatilho** por item ("só entra o que tem um 'quando
X aparecer'"), porque melhoria sem gatilho é desejo.

Primeiro item da lista nova: **M-Q, overlay de cromo**. A barra de abas, os
cartões da sidebar e o toast são desenhados com retângulo + glifo (herdam a
fonte, a paleta e o alinhamento de célula de graça, e não puxam toolkit — é o
motivo do projeto existir), mas cada um é escrito duas vezes: o desenho em
`render.rs` e o teste de clique em `app.rs`, com as mesmas coordenadas em
fórmulas separadas. Três bugs desta semana foram essa divergência. O item é
transformar o cromo numa lista de elementos com hit-test declarado, no mesmo
`wgpu`, quando aparecer o quarto elemento — antes de escrever o par
desenho/clique pela quarta vez.

## 2026-11-02 — a fronteira: painel = paridade, backlog = atividade nova

Regra de natureza, agora escrita nos dois arquivos: **se o Python faz, é
paridade e mora no `MIGRATION.md`; se não faz, é atividade nova e mora no
`BACKLOG.md`**. Um trabalho tem um dono só.

O que a auditoria achou: **44 das 252 linhas do painel não eram paridade** — 25
delas ainda na fila (`falta`, contando como trabalho a fazer) e 19 já feitas mas
marcadas como melhoria. E 13 dos 15 itens de backlog já tinham linha no painel,
com o mesmo trabalho em dois estados. Pior: duas duplicações dentro do próprio
`MIGRATION.md` — a seção 15 era uma segunda cópia do painel, congelada 9 itens
atrás, e a seção 16 era a fila de melhorias.

| antes | depois |
|---|---|
| 252 linhas, 25 delas melhoria na fila + 19 melhoria feitas | **208 linhas, só paridade** (145 feito / 14 parcial / 46 falta / 3 não) |
| 15 itens de backlog, 13 duplicando o painel | **19 atividades novas** + 17 fechadas, nenhuma com linha no painel |
| 67,9% "equivalente pronto" contando extras | **73,1% de paridade** (o que é atividade nova não entra na conta) |

Duas linhas do backlog eram paridade disfarçada e voltaram para o painel:
`M-A` (dano por linha — o Python faz, `view.py:330-364`) e `M-G` (kitty keyboard
— o Python faz em `kitty_kbd.py`; sobrou só a flag 4 como atividade nova).

Seis linhas da fila ficaram órfãs ao sair do painel e ganharam item próprio:
`M-R` (OSC 52/7/133), `M-S` (F13-F24 e `KP_*`), `M-T` (`?1016`), `M-U` (DECALN
e combinantes) e `M-V` (menu de contexto, que é o gatilho declarado do `M-Q`).

## 2026-11-02 — atividade nova que já estava no programa

Consolidação, não trabalho novo: o que o Python **não** faz e que já foi feito
aqui sai do painel de paridade e fica registrado só nesta linha. A verificação é
a que rodou em cima delas — `cargo test` (52 no `bzmux-core`), `--selftest`
14/14, `mc`/`vim`/`dmesg` no roteiro manual, mais as provas específicas citadas
(item a item, quando houve uma).

**Parser e grade:** charset G1 com SO/SI; wide char (`wcwidth`) com célula de
continuação; tab stops configuráveis (HTS/TBC/CHT/CBT); origin mode `?6` e
autowrap desligável `?7`; DCS/APC/PM/SOS consumidos até ST (no Python vazam na
tela); DECSC salvando SGR/origin/autowrap e não só o cursor; `ESC D`/`ESC E`
(IND/NEL, no-op no Python); clamp de `n` em IL/DL/ICH/DCH/ECH; SU/SD; REP
(`CSI b`); teto de comprimento nos estados `csi`/`osc`.

**Cores e desenho:** SGR completo (itálico, faint, blink, riscado, overline, cor
e estilos de sublinhado) — provado com `bat` e `git diff`; OSC 4/104/110/111/112
como *set* em runtime; paleta/tema configurável, com `color0..15`/fg/bg/cursor
no arquivo — conferido por amostragem de pixel contra o `tortty-python` (`mc` e `htop`
idênticos).

**Entrada:** roda com delta fracionário (trackpad) acumulando; `?1005`
coordenada em UTF-8 acima do teto de 222 (testes de unidade em `mouse.rs`).

**Janela e CLI:** grid travado pelo socket sobrevive ao arredondamento do WM
(laço de correção `dw/dh`); `--cols`/`--rows`/`--dpi`/`--no-vsync`/`--prof`
(conferido com `stty`: 100x10 pedido, 10 100 na saída).

**Ferramenta:** `--selftest` no binário (14 checagens sem display nem GPU), no
`check.sh` e no CI.

## 2026-11-02 — teclado do kitty (E12-E15, E17-E20, B13)

O filho liga flags por `CSI > u` e as teclas mudam de forma. As flags moram no
`Term` (uma por aba, empilháveis) e o encoder só consulta: **estado no core,
codificação em `keys.rs`** — a mesma divisão do `kitty_kbd.py`.

| flag | o que muda |
|---|---|
| 1 (disambiguate) | ESC vira `CSI 27u`, Ctrl+a vira `CSI 97;5u`, seta com modificador vira `CSI 1;5A`; seta sem modificador continua no legado (e respeita DECCKM) |
| 2 (event types) | `:1`/`:2`/`:3` — tecla segurada deu `CSI 97;1:2u` cinco vezes e `CSI 97;1;3u` na soltura |
| 8 (all keys) | `a` vira `CSI 97u`; Shift+letra deixa de ser texto puro |
| 16 (text) | `CSI 97;97u`, com o texto depois do código |

Duas coisas que só apareceram com o teclado de verdade:

1. **A soltura precisa chegar ao encoder.** O winit entrega `Released` e o app
   descartava em `if event.state == Pressed`. Com a flag 2, o release é
   informação. Mas atalho local não pode rodar duas vezes (Ctrl+Shift+N abriria
   duas abas) — daí o atalho ficar só no aperto, e o release seguir sozinho
   para o encoder.
2. **Atalho comido come a soltura.** `Ctrl+Shift+1` trocava de workspace e
   deixava `CSI 49;1:3u` no filho: o aperto era do app, a soltura vazava. E o
   X11 solta **Shift e Ctrl antes** da tecla, então o marcador do aperto
   consumido não pode ser limpo por elas (foi o `take()` cego que causou o
   vazamento na primeira tentativa).

Desvio conhecido, e é do Python também: Shift+letra sai como `CSI 65;2u` (o
caractere já vem depois do shift) e não `CSI 97;2u` como no kitty — a tecla base
exigiria uma tabela de layout. F3 saiu como `CSI 1;2R` (forma do kitty) e **não**
`CSI 13;2~`: o `kitty_kbd.py` do Python tem F3 como `13 ~`, que é Shift+F3 do
xterm.

Verificação: `state`/`text` por socket com um filho em modo raw que imprime o
hex do que recebe — as respostas de consulta (`?0u`, `?3u`, `?11u`) e as teclas
(`97u`, `97;5u`, `97;1:2u`, `97;1:3u`, `27u`, `1;5A`, `1;2R`) lidas da tela, e
o atalho local sem deixar rastro no filho.

## 2026-11-02 — automação sem `xdotool` (L2, L3, L5, L7, L8, L12, L15, J7-J9, K15)

O que faltava para o `benchrun.py` e o CI não dependerem de mouse e de captura
de tela. Nada de GPU aqui: é instrumentação.

**`state` completo.** Ganhou `xid` (do `RawWindowHandle`, Xlib/Xcb; 0 no
Wayland — quem vai capturar pixel pergunta ao app, não ao X), `config`,
`errors` (o erro de config é fato mesmo quando a config anterior é mantida),
`dpi`, `advance`, `padding` e `pin`. `text` já tinha `screen`; ficou conferido.

**`keys` pelo caminho do teclado de verdade.** O verbo agora passa pelo atalho
local **antes** do encoder, como a tecla física: `ctrl+shift+n` responde
`handled:true` e não manda byte nenhum (`ctrl+a` continua `01`). Antes ele
falava direto com o encoder, e a resposta era mentira sobre o que o app faria.

**`prof` com escrita.** `writes`, `write_kb` e `eagain`, contados na única
porta de escrita do pty — que virou um `pty_write` no app (antes eram seis
`pty.write` espalhados). `log` liga, desliga (`off`) e troca de arquivo em
execução.

**Dump e ciclo de vida.** `--dump-to`, `--quit-after SEG`, `SIGUSR1` (dump na
hora) e `SIGTERM` (dump e saída limpa). O handler de sinal só levanta um
`AtomicBool`: quem age é o laço, que já acorda a cada 250 ms. O dump tem
cabeçalho por workspace e por aba, scrollback (as últimas 40 linhas) e o
viewport, marcando `[fundo]` em quem não é a aba ativa — conferido com dois
workspaces e duas abas: 78 linhas, três cabeçalhos, processo fora depois do
`SIGTERM`.

**Socket órfão (L12).** Antes, qualquer caminho ocupado era apagado no
arranque; agora arquivo que não é socket nunca é tocado (`caminho existe e não
é socket`) e o órfão só sai quando o `connect` recusa. Verificado com um
arquivo de texto no lugar do socket: sobreviveu com o conteúdo intacto.
