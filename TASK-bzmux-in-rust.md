# TASK — bzmux em Rust (experimento de viabilidade)

Documento de andamento deste experimento. Fonte analisada:
`/home/user/tortty-python` (Python puro, GTK3 + cairo + Pango).
Alvo aqui: `rust_term/` (workspace Cargo).

Estado: **em andamento**. Cada seção marcada como `[x]` foi medida, não suposta.

---

## 1. Objetivo

Responder com número, não com opinião, a duas perguntas:

1. **Distribuição** win/lin/mac fica fácil em Rust?
2. **Desempenho** fica melhor que o Python (que hoje faz ~114 fps / 4,9 MB/s no
   doom-fire a 61×18 no display real, e ~700 fps no kitty)?

Condição de sucesso do experimento (o "ir/não ir"):

- [ ] projeto Rust que abre janela, roda shell em pty e desenha a grade;
- [ ] roda o doom-fire de forma comparável (mesma grade, mesmo gerador);
- [ ] benchmark headless do parser (µs/célula) e de ponta (fps, MB/s);
- [ ] funciona em `vim`, `mc` e fluxo de console (`dmesg`) — requisito de
      utilidade, não de performance;
- [ ] debug socket com o mesmo protocolo JSON por linha (instrumentação/testes);
- [ ] comparação lado a lado com o bzmux Python na mesma máquina e display.

Fora de escopo desta fase: abas/workspaces, imagens do protocolo kitty,
configuração em arquivo, clipboard/seleção, mouse reporting, reflow, kitty
keyboard protocol, Windows/macOS de verdade (só o desenho da arquitetura).

---

## 2. Análise do projeto de origem

### 2.1 O que foi medido lá (docs/BENCHMARKS.md)

- Carga principal: `tools/doomfire.py`. Buffer de intensidade 0..36 propagado de
  baixo para cima; desenho em meio-bloco `▀` (dois pixels por célula), com
  `38;2;r;g;b` + `48;2;r;g;b` truecolor **por célula**, sem agrupar. É o pior
  caso do parser e do desenho: ~34–38 bytes de escape por célula.
- Número de ponta do Python (2026-11-01, ``:0` com glamor, 61×18, 1200 frames):
  **113,9 fps / 4,9 MB/s / 42,8 KB por frame / draw 13,9 ms**. Em Xvfb: 129 fps e
  draw 4,2 ms (o display muda o número por 3,4×).
- Referência externa: kitty 0.48.2 a 70×21 faz 676–714 fps / 38–41 MB/s. O
  Python faz 5,2 µs por célula; o kitty 1 µs.
- Decomposição do parse no Python: **35 µs/célula**, dos quais **91% é SGR
  truecolor** (fechar o CSI, `split(';')`, `int()`, montar listas). O P20 trouxe
  um fast path com regex + cache `escape → cor` e caiu para 5,5 µs/célula (5,8×).
- Decomposição do desenho: ~5,7 µs por run de cor; `show_layout` do Pango 43%,
  fundo+estado (4 chamadas de contexto cairo por run) 27%.
- Descoberta central do último ajuste: **o caro é alternar fundo e glifo**
  (13,0 ms → 7,9 ms só agrupando em duas passadas), porque cada operação troca
  estado no servidor X. Um renderizador por instância (GPU) elimina a alternância
  por construção.
- Parse isolado (sem GTK) de `dmesg` (1319 linhas): 42,6 ms sem cor / 78,6 ms com
  cor. É o que congela a UI ao despejar log no thread do GTK.

### 2.2 O que é indispensável para o benchmark e para vim/mc/dmesg

Do código Python (`bzmux/term.py` 1126 linhas, `view.py`, `palette.py`):

| área | o que precisa existir | por quê |
|---|---|---|
| grade | `Cell {ch, fg, bg, flags}`, cursor, autowrap, `row_wrapped` | base de tudo |
| SGR | 16/256/truecolor, subparâmetro `38:2::r:g:b`, bold/underline/inverse | doom-fire inteiro; vim com `termguicolors` |
| CSI de edição | ED/EL, IL/DL, ICH/DCH/ECH, DECSTBM (região de scroll), CUP/CUU/CUD/CUF/CUB/CHA/VPA | vim, mc redesenham com isso |
| modos | `?1` DECCKM (setas `\EOA`), `?25` cursor, `?7` autowrap, `?47/1047/1049` alt-screen (+ limpar scrollback no 1049), `?1000/1002/1003/1006` mouse, `?1004`, `?2004`, `?2026` | mc e vim vivem de alt-screen + setas em modo aplicação |
| charset DEC | `ESC ( 0` → gráficos de linha (`lqk x m j`) | **mc desenha as molduras assim**; sem isso vira `lqqqqk` |
| consultas | DA1 (`\e[?62c`), DA2 (`\e[>1;100;0c`), DSR/CPR (`\e[6n`), DECRQM (`$p`), DECSCUSR (`6 q`), window ops 16/18 (célula em px) | apps de TUI esperam resposta e travam o startup sem ela |
| OSC | título (0/2), consumir 4/8/10/11/52 sem quebrar | shell, vim, `dmesg` |
| scrollback | histórico com teto, `view_off` (roda do mouse) | fluxo de console |
| pty | `openpty` + `fork/exec` + `TIOCSWINSZ` + leitura não-bloqueante | — |
| teclado | keyval → bytes, DECCKM, Alt como prefixo ESC, Shift+Tab `\e[Z`, Ctrl+letra | vim/mc |

Do que **não** precisa nesta fase: imagens kitty (`graphics.py`), abas/workspaces
(`window.py`), reflow (`_reflow`), seleção/clipboard, config em arquivo
(`config.py`), protocolo de teclado kitty (`kitty_kbd.py`), atalhos e cromo.

### 2.3 Protocolo do debug socket (replicar igual)

`bzmux/debugsock.py`: socket Unix, **uma linha JSON por pedido e por resposta**,
`{"ok":true,...}` ou `{"ok":false,"err":...}`, só mesmo uid
(`SO_PEERCRED`), `0600`. Verbos: `state`, `text`, `write`, `keys`, `resize`,
`prof`, `log`, `quit`. `--debug-socket auto` = `$XDG_RUNTIME_DIR/bzmux-<pid>.sock`.

O `tools/benchrun.py` depende disso: ele sobe o app, pergunta o grid, usa
`resize` até bater no alvo (61×18), e **só então** digita a carga no pty com
`write`. Manter o protocolo igual permite reusar o mesmo harness nos dois
terminais — que é o que torna a comparação honesta.

---

## 3. Respostas às perguntas do usuário

### 3.1 Distribuição win/lin/mac fica fácil? — **Sim, substancialmente.**

O que o Python carrega hoje (`docs/PORTABILITY.md`): PyGObject **não tem wheel em
lugar nenhum** (só sdist → compilar contra headers), pycairo não tem wheel de
macOS, Pango nunca será pip-installable, e o Qt tem dependência de sistema no
Linux (`libxcb-cursor0`). Resultado: instalar = conda (~350–400 MB), ou rezar
para o desktop ter GTK.

Em Rust a pilha escolhida é toda nativa e sem dependência de sistema:

- janela/eventos: `winit` → X11 e Wayland nativos, Win32, Cocoa;
- GPU: `wgpu` → Vulkan/DX12/Metal (e GL como fallback) sem código por plataforma;
- fonte: `fontdb` + `swash` — rasterizamos o glifo em processo. **Sem freetype,
  sem fontconfig, sem Pango**;
- pty: `portable-pty` → pty Unix e **ConPTY** no Windows (é a razão de escolher
  esse crate e não `openpty` cru);
- texto: `unicode-width`; nada de ICU.

Consequência concreta: um binário único de ~5–15 MB por alvo, gerado por CI
(`cargo build --target x86_64-pc-windows-msvc`, `aarch64-apple-darwin`, etc.).

Ressalvas que **não** desaparecem (é por isso que a resposta é "sim, mas"):

1. **Assinatura/notarização no macOS**: binário não assinado abre com aviso do
   Gatekeeper; distribuição limpa exige conta Apple (US$ 99/ano) + `codesign` +
   `notarytool`. Rust não muda isso — é política do SO.
2. **Windows SmartScreen** idem para binário novo sem reputação.
3. **Linux**: X11/Wayland ok, mas o número "roda em qualquer distro" pede build em
   glibc antiga (ou musl) e dependência de driver gráfico; Vulkan não existe em
   máquina velha → precisa do fallback GL (`wgpu` cobre, mas é caminho a testar).
4. Fontes: como não usamos fontconfig, no Windows/macOS lemos as fontes do
   sistema; um fallback embutido resolve máquina sem monoespaçada decente.

Comparação honesta: Python→Rust troca "levar 350 MB de stack GTK" por "levar um
binário", e troca três backends de janela escritos à mão por um crate. O que
**continua** sendo trabalho: assinatura, CI matriz, e testar cada plataforma.

### 3.2 Desempenho fica melhor que o Python? — **Sim, e a diferença é estrutural.**

Não é "Rust é mais rápido" genérico: os dois gargalos medidos no Python
desaparecem por construção.

| gargalo medido no Python | custo hoje | por que cai em Rust |
|---|---|---|
| fechar e interpretar CSI truecolor | 32 de 35 µs/célula | parsing é O(bytes) sem alocação: `split` sobre slice + `u32` na stack. Ordem de grandeza: dezenas de **ns** por escape |
| alternar `fill` de fundo e `show_layout` por run (troca de estado no servidor X) | 13,0 ms/frame; cai para 7,9 só agrupando | uma **única** chamada de desenho instanciada: fundo e glifo viram o mesmo pipeline, mesma passada, ordem garantida por instância |
| shaping/layout por run de cor | 43% do draw | nenhum shaping por frame: glifo já está no atlas, o frame é só montar vértices |
| célula Python (lista de 4 objetos) + `Vec` de listas | ~100 ns por acesso, GC | `#[derive(Copy)] Cell` em `Vec<Row>` contígua |

Estimativa (marcada como estimativa): parse de 35 µs/célula (Python sem o P20) →
~0,02–0,05 µs/célula; com o P20 (5,5 µs) → ~0,02–0,05 µs também, porque em Rust
o fast path vira o caminho normal. Desenho de 13,9 ms/frame → montagem de ~2200
instâncias (~30 µs) + GPU. Ou seja **>10× no parse e >50× no desenho**, o que
muda o limite do sistema de "Python desenhando" para "vsync do monitor".

Duas ressalvas que valem mais que o fps:

1. Em uso real o fps é limitado pelo vsync (60–144 Hz). O ganho visível é
   **latência** e **headroom de CPU** (despejar `dmesg` não congela mais a UI),
   não o número do doom-fire.
2. `wgpu`/GPU paga um custo fixo por frame. Em grade pequena o gargalo passa a
   ser o gerador; por isso a comparação tem de ser na mesma grade e com o mesmo
   `tools/doomfire.py` — senão mede-se o gerador, não o terminal.

### 3.3 Veredito

As duas respostas são "sim". **O experimento segue**, com uma condição de
honestidade: os números da seção 7 são medidos aqui e agora, na mesma máquina,
mesmo display e mesma grade do Python; se o Rust não ganhar de forma clara, o
documento diz isso.

### 3.4 Decisões tomadas (usuário, 2026-11-01)

1. **Rota C — Rust completo.** Não é o híbrido PyO3 nem otimizar o Python.
2. **Repo novo** (não dentro do `tortty-python`).
3. **Só Linux nesta fase**; `portable-pty` já escolhido para o dia do Windows
   (ConPTY), mas nada de CI de outras plataformas agora.
4. **Vsync ligado por padrão**, `--no-vsync` para benchmark.
5. **Fonte em pontos**, métrica equivalente ao Pango — mas sem gambiarra: se a
   conversão pt→px do `swash` divergir do Pango, registra-se a diferença e a
   comparação continua travada em células.
6. **Sem reflow na fase 1** (vim/mc redesenham no SIGWINCH; histórico quebrado ao
   estreitar é efeito aceito e documentado).
7. **Critério de abortar:** se o Rust não fizer **≥3× o MB/s do Python na mesma
   grade** e não abrir `vim`/`mc` corretamente, o experimento não paga o custo de
   migração e a recomendação passa a ser a rota B (núcleo Rust + UI Python).

## 3.5 Distribuição: o que "fácil" significa na prática

Pergunta do usuário: dá para copiar o executável e rodar no Mac? É loja paga?
Release no GitHub funciona?

- **Copiar entre sistemas não**: binário Linux é ELF; macOS é Mach-O; Windows é
  PE. "Copiar e rodar" vale dentro do mesmo sistema (e é aí que o Rust ganha do
  Python: binário autocontido, sem GTK/Python/fontconfig). Para o Mac é preciso
  compilar **para** o Mac.
- **Como compilar para o Mac**: runner `macos-14`/`macos-15` (arm64) e `macos-13`
  (x86_64) no GitHub Actions; `lipo` junta num universal. Cross-compile do Linux
  com `osxcross` é possível e frágil (SDK da Apple) — não é caminho.
- **Gatekeeper**: o bloqueio vem do atributo `com.apple.quarantine`, que o
  **navegador** marca no download. `curl`, `git clone` e `brew` não marcam → roda
  direto. Download por Safari/Chrome de um zip sem notarização é bloqueado, e no
  macOS 15+ o antigo "clique direito → Abrir" foi removido (o usuário precisa ir
  em Ajustes → Privacidade e Segurança → "Abrir Mesmo Assim").
- **O que é pago**: não é loja, é o **Apple Developer Program (US$ 99/ano)**, que
  emite o certificado *Developer ID Application* necessário para **notarizar**.
  A notarização em si é gratuita; a conta é que não. Só isso remove o aviso no
  download por navegador. App Store é irrelevante (sandbox quebra pty).
- **Homebrew tap** é a rota amigável no Mac: gratuito, baixa por `curl` (sem
  quarentena), dá `brew upgrade`.
- **Windows**: `.exe` sem assinatura gera aviso do SmartScreen ("Mais informações
  → Executar assim mesmo"); certificado OV/EV hoje exige token/HSM.
- **Conclusão**: Release no GitHub funciona nos três. Linux perfeito; macOS limpo
  via `curl`/Homebrew (US$ 99/ano só para abrir sem aviso vindo do navegador);
  Windows com aviso do SmartScreen. Nenhuma taxa por download, nenhuma loja.
- **Plano de empacotamento** (fase futura, não desta): matriz de CI com
  `bzmux-linux-x86_64`, `bzmux-macos-arm64`, `bzmux-macos-x86_64`, universal via
  `lipo`, `bzmux-windows-x86_64.exe` + `SHA256SUMS` em Release.

---

## 4. Arquitetura escolhida

```
rust_term/
├── TASK-bzmux-in-rust.md      (este arquivo)
├── Cargo.toml                 workspace
├── crates/bzmux-core/         emulação pura: célula, paleta, parser, grade
│   ├── src/{lib,color,cell,grid,charset,term}.rs
│   └── src/bin/parsebench.rs  benchmark headless do parser (µs/célula)
├── crates/bzmux/              app: janela, GPU, pty, teclado, debug socket
│   └── src/{main,cli,app,keys,pty,prof,debugsock,render,glyph,atlas}.rs
└── tools/                     doomfire.py, benchrun.py, bench.sh (comparação)
```

Regra herdada do Python e mantida: **`bzmux-core` não conhece GUI** (nem winit,
nem wgpu, nem pty). Isso é o que permite `cargo test -p bzmux-core` rodar sem
display — e é o que garante que o benchmark de parser mede o parser.

Decisões com motivo:

- **Parser próprio**, não `vte`/`alacritty_terminal`: o bzmux nasceu de um parser
  VT próprio e a compatibilidade (kitty/ghostty) é o produto. Dependência de
  terceiro aqui travaria exatamente o que se quer evoluir.
- **GPU (`wgpu`) em vez de cairo/softbuffer**: o gargalo medido do Python é troca
  de estado por operação de desenho. Instanciar quad é o oposto disso.
- **Um pipeline só** para fundo, glifo, sublinhado e cursor: o atlas guarda um
  pixel branco; fundo é um quad "sólido" e glifo é um quad com UV na máscara de
  cobertura. Uma draw call por frame, ordem de blending garantida pela ordem das
  instâncias.
- **Atlas de glifos sob demanda** (`swash`): rasteriza 1× por (face, glifo,
  tamanho), nunca por frame.
- **`portable-pty`**: é o que dá ConPTY de graça; `openpty` cru custaria uma
  implementação inteira no dia em que o alvo Windows entrar.

---

## 5. Fases

- [x] 1. Análise do projeto Python (term/view/palette/debugsock/BENCHMARKS/PORTABILITY)
- [x] 2. Workspace + `bzmux-core`: célula, paleta 256, charset DEC, grade
- [x] 3. Parser VT: ground/CSI/OSC/ESC, SGR truecolor, modos, consultas
- [x] 4. Testes de núcleo (23 testes) + `parsebench` headless
- [x] 5. App: pty (`portable-pty`), janela (`winit`), teclado, resize
- [x] 6. Renderer GPU (`wgpu`): atlas de glifos (`swash`/`fontdb`) + instâncias
- [x] 7. Debug socket (mesmo protocolo JSON; verbos state/text/write/keys/resize/prof/log/quit)
- [x] 8. Rodar doom-fire e medir; comparar com o Python (seção 7)
- [x] 9. Teste funcional vim / mc / dmesg
- [ ] 10. Conclusão e recomendação (se seguir migrando / repensar)

## 6. Protocolo de medição (para o número valer)

- Mesma máquina, mesmo display (`:0`), um terminal de cada vez, governor
  registrado (`schedutil`).
- Grade **travada em células** (61×18 e 70×21) via verb `resize` do debug socket,
  conferida antes de digitar a carga — como o `tools/benchrun.py` do Python.
- Mesmo gerador (`tools/doomfire.py`, `--fps 0`) rodando **dentro** de cada
  terminal: o que se mede é o consumidor, não o gerador.
- Três números por rodada: fps (do gerador), MB/s, e o custo interno (parse/draw
  por chamada) do próprio app.
- Parser isolado: `parsebench` com o frame exato do doomfire, reportando µs/célula
  e MB/s, repetindo até estabilizar (melhor de N).
- Números antigos de outro dia/governor/grade não entram na tabela.

---

## 7. Resultados

Medido nesta máquina (Ryzen 7 4800H, Radeon Renoir/RADV, X11 `:0`), governor
`schedutil` a 4,1–4,3 GHz, um terminal de cada vez, 2026-11-01. Célula medida:
Python 14x28 px, Rust 14,13x27,32 px (mesma fonte de 13 pt, mesmo DPI efetivo —
a conversão pt→px do `swash` bate com a do Pango dentro de 2,4% na altura).

### 7.1 Parser headless (`parsebench`, sem GUI, sem display)

Frame exato do doom-fire (61x18, 1098 células, 35 B/célula = 2 SGR truecolor +
`▀` por célula), melhor de 800 rodadas:

| parser | µs/célula | MB/s | ns/escape |
|---|---|---|---|
| Rust (este) | **0,183** (média 0,199) | 180–195 | 99 |
| Python pós-P20 (o de produção) | 5,49 | ~7 | — |
| Python pré-P20 | 35,0 | ~1 | — |

Ganho: **30x** sobre o Python de produção, **191x** sobre o Python sem o fast
path. A referência do kitty (medida no benchmark de ponta, com desenho) é ~1,0
µs/célula.

Carga real, `dmesg` do sistema (1397 linhas, 109 KB sem cor / 134 KB com cor),
grade 100x40, melhor de 20 passadas:

| payload | Rust | Python | ganho |
|---|---|---|---|
| `dmesg` sem cor | **2,11 ms** (51,7 MB/s) | 42,6 ms | 20x |
| `dmesg --color=always` | **2,22 ms** (60,2 MB/s) | 78,6 ms | 35x |

É este número que explica a UI do Python "congelar" ao despejar log: 42–79 ms
de trabalho no thread da UI contra 2,2 ms.

### 7.2 Ponta a ponta (doom-fire dentro do pty, 1200 frames)

Comando: `tools/benchrun.py` (mesmo protocolo: socket, `resize` até o grid
bater, confere, depois digita a carga). Rust com `--no-vsync` — com vsync o
número seria o do monitor, não o do terminal.

| grade | terminal | fps | MB/s | `draw` por chamada |
|---|---|---|---|---|
| 61x18 | Rust | **916,6–968,2** | **39,4–41,6** | 0,25–0,28 ms |
| 61x18 | Python | 113,2–120,9 | 4,9–5,2 | 7,77–8,11 ms |
| 70x21 | Rust | **628,6–666,9** | **35,8–38,0** | 0,28–0,30 ms |
| 70x21 | Python | 62,3–70,2 | 3,6–4,0 | 13,58–13,73 ms |

- **8,0–9,0x em fps e MB/s a 61x18**; **9,5x a 70x21**. O critério de abortar
  (≥3x) foi batido com folga.
- O desenho caiu **29x** (61x18) e **46x** (70x21): 7,8–13,7 ms → 0,25–0,30 ms.
  A causa é a que o BENCHMARKS do Python já apontava: não existe mais
  "alternar `fill` de fundo e `show_layout` por run" — fundo, glifo, sublinhado e
  cursor são instâncias do mesmo pipeline, numa draw call por frame.
- **O limite agora é o gerador, não o terminal**: o doom-fire isolado faz ~55
  MB/s (756 fps a 80x24, medido no projeto Python). A 39–42 MB/s o Rust está a
  ~75% do teto da carga. fps cresce 9x mas MB/s não cresce 9x pelo mesmo motivo —
  a tela de 61x18 não tem mais células para consumir.
- `parse` saiu em 0,13–0,15 ms por chamada e `pty` (só a leitura) em 0,26–0,39
  ms; no Python o `on_pty` (leitura + parse) era 0,28 ms. Ou seja: o parse
  praticamente desapareceu do orçamento do frame.

### 7.3 Funcional (o que foi aberto e conferido na tela)

- **`mc`**: painéis, molduras em DEC Special Graphics (`lqk x m j` → `┌┐└┘│─`),
  cores, barra de F-keys, alt-screen e setas em modo aplicação (DECCKM).
- **`vim`**: arquivo real com `-u NONE`, títulos `~`, status line, cursor de
  bloco, navegação por setas/Home/End pelo caminho real do teclado
  (XTest → winit → encoder → pty).
- **`dmesg --color=always`**: cores, rolagem, 287 linhas de histórico, roda do
  mouse subindo no scrollback (`view_off` = 12 linhas depois de 6 cliques).
- **Truecolor por célula**, negrito e SGR 256 conferidos por captura de tela.
- **Debug socket**: `state`, `text`, `write`, `keys`, `resize`, `prof`, `log`,
  `quit` — o mesmo harness de medição do Python roda nos dois.

### 7.4 Pegada

- Binário release: **10,1 MB**, ligado só em `libc`, `libm`, `libgcc_s` (+ o
  driver gráfico em runtime). Sem GTK, sem Pango, sem freetype, sem fontconfig,
  sem Python.
- Startup até abrir janela e rodar comando: **0,18 s**.
- `cargo build --release` do zero: ~2 min (a maior parte é `wgpu`).

## 8. Riscos e o que fica pendente de propósito

Pendente nesta fase (nada disso é requisito do experimento):

- sem reflow no resize (decisão 6); o histórico "quebra" ao estreitar, e
  vim/mc redesenham no SIGWINCH;
- sem abas/workspaces, imagens do protocolo kitty, seleção/clipboard, mouse
  reporting (`?1000/1002/1003/1006` já são lidos e guardados, mas não emitem),
  links OSC 8, config em arquivo, protocolo de teclado do kitty (`CSI u`);
- `IM` (insert mode) e `LNM` ignorados; combining marks não compõem (mesma
  limitação do Python);
- `ScaleFactorChanged` em runtime não recria a fonte (só no arranque).

Riscos medidos ou conhecidos:

- Vulkan ausente em máquina antiga → falta exercitar o backend GL do `wgpu`
  (`WGPU_BACKEND=gl` existe como válvula, não foi testado);
- a variável de ambiente `WGPU_BACKEND`/`WGPU_ADAPTER_NAME` permite forçar
  llvmpipe; um teste em software renderer é o próximo passo de portabilidade;
- fonte: como não há fontconfig, a escolha é lista de preferência + fallback
  genérica. Em máquina sem nenhuma monoespaçada o app sai com mensagem;
- `--no-vsync` é obrigatório para medir vazão: com vsync o número é o do
  monitor (60–144), o que é o comportamento correto em uso.

Conclusão da fase (preencher no item 10 depois da revisão do usuário).
