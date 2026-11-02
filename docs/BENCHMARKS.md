# Benchmarks e medição

**Este é o único arquivo com número de vazão, parse, desenho e pegada.** Os
outros docs citam o fato em uma frase e linkam para cá. Todo número abaixo tem
data, condição e governor: número sem os três não é comparável, e é assim que
dois terminais diferentes viram uma tabela honesta.

## Convenções

- **Condição travada em células, não em pixels.** O `tools/benchrun.py` sobe o
  app com o debug socket, pede `resize` até `state` confirmar o grid alvo, e só
  então digita a carga. Se o grid não bater, ele sai com código 3 e não imprime
  número: medida de outra condição é ruído com cara de resultado.
- **A carga é o mesmo arquivo nos dois terminais.** `tools/doomfire.py` é cópia
  byte a byte do gerador do repo Python (mesmo `sha1`), então o que se mede é o
  consumidor.
- **Sem vsync para medir vazão.** O app aceita `--no-vsync`; com vsync o número
  é o do monitor (60–144 Hz), que é o comportamento correto em uso. Isso é
  registrado no resultado, não escondido.
- **Um terminal por vez**, nenhuma outra janela animando, `schedutil`.
- **Mediana de 3 tiros** de 1200 frames (o `best`/`média` do parsebench é
  melhor de N passadas, e está escrito na linha).

## Máquina

| | |
|---|---|
| CPU | Ryzen 7 4800H, 16 threads, governor `schedutil` |
| GPU | Radeon Renoir integrada, RADV (Vulkan 1.4) |
| Sessão | X11 `:0`, sem compositor redirecionando |
| RAM | 22 GB, sobra >10 GB |
| Data | 2026-11-01 |

Frequência amostrada durante a medição (a cada 200 ms, pico de todos os cores):
Rust 3,66–3,95 GHz médio / 4,29 GHz pico; Python 4,11–4,21 GHz médio / 4,29 GHz
pico. Ou seja: **o Rust mediu com clock menor e ganhou mesmo assim.**

`scaling_governor` importa: com `powersave` o número cai pela metade e a
comparação mente. Confira `cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor`
antes de acreditar em qualquer linha abaixo.

## Carga principal: doom-fire

O `doomfire.py` pinta a tela inteira por frame escrevendo, por célula,
`ESC[38;2;r;g;bm ESC[48;2;r;g;bm ▀` — dois truecolor e um meio-blocos por
célula. É carga de pior caso de propósito: SGR truecolor é o escape mais caro, e
o `▀` faz cada célula precisar de duas cores.

### Como rodar

```sh
tools/benchrun.py                                   # Rust, 1200 frames, 61x18
tools/benchrun.py --app ~/tortty-python/run.sh   # referência Python
tools/bench.sh                                      # a escada inteira, as duas grades
tools/bench.sh --frames 300 --shots 1 --so-rust     # fumaça
```

Uma linha do `benchrun.py`:

```
app rust
grade 61x18
fps 968.2
mbs 41.6 MB/s
bpf 42286
draw 0.28 ms/call (2339 chamadas)
parse 0.13 ms/call (2325 chamadas)
pty 0.31 ms/call (5367 chamadas)
cell [14.131470680236816, 27.323135375976562]
ghz 3.66/4.29
```

`draw` é o tempo dentro do `RedrawRequested` (montar instâncias + submeter);
`parse` é o `Term::feed`; `pty` é só a leitura do fd (o parse é da thread
principal, de propósito: um core faz os dois e o frame não espera lock).

### O número (doom-fire, ponta a ponta, 1200 frames)

Medição de 2026-11-02, depois da F1 (mesma máquina, mesmo protocolo, mesma
largura de célula). A coluna `draw` de hoje inclui a decoração nova
(sub/sublinhado por estilo, riscado, overline) e não mudou.

| grade | terminal | fps | MB/s | `draw` por chamada |
|---|---|---|---|---|
| 61x18 | **Rust** | **970–1005** | **41,7–43,2** | 0,29–0,33 ms |
| 61x18 | Python (pós-P20) | 112–143 | 4,8–6,1 | 8,0–8,2 ms |
| 70x21 | **Rust** | **658–732** | **37,5–41,7** | 0,32–0,34 ms |
| 70x21 | Python (pós-P20) | 63–72 | 3,6–4,1 | 13,1–13,4 ms |

Ganho: **7–9x (61x18) e 9–11x (70x21)** em fps e MB/s; **25–46x** no desenho.

Medição anterior (2026-11-01, antes da F1), para referência de deriva: Rust
917–968 fps / 39,4–41,6 MB/s em 61x18 e 629–667 / 35,8–38,0 em 70x21. A
diferença é ruído de máquina, não mudança de desenho.

Célula medida nos dois: 13,9–14,1 x 27,3–28,0 px (fonte 13 pt, mesmo DPI
efetivo). A conversão pt→px do `swash` bate com a do Pango dentro de 2,4% na
altura — por isso a comparação em células é justa, e a diferença de pixels por
célula (2%) não explica 9x.

### O teto agora é o gerador

O doom-fire isolado faz **~55 MB/s** (756 fps a 80x24, medido no repo Python).
A 42–43 MB/s o Rust está a ~78% do teto da carga. É por isso que o fps cresce
9x mas o MB/s não cresce 9x: a tela de 61x18 não tem mais células para consumir;
quem ainda sobra é o Python do gerador. Medir mais rápido que isso exige gerador
em C ou reusar buffer — e aí se mede o gerador, não o terminal.

## Parser isolado (`parsebench`, sem display)

Mesmo frame do doom-fire, melhor de 800 rodadas, célula de 35 B:

| parser | µs/célula | MB/s | ns/escape |
|---|---|---|---|
| **Rust** | **0,183** (média 0,199) | 180–195 | 99 |
| Python pós-P20 (o de produção) | 5,49 | ~7 | — |
| Python pré-P20 | 35,0 | ~1 | — |
| kitty (referência do benchmark Python) | ~1,0 | ~10 | — |

**30x** sobre o Python de produção; **191x** sobre o Python sem o fast path.
O kitty está na tabela como referência de grandeza — o número dele foi medido
no repo Python, com desenho junto, então não é a mesma régua.

Por que o Python custava 35 µs: 91% era SGR truecolor (fechar o CSI,
`split(';')`, `int()`, montar listas). Em Rust o SGR truecolor é uma função que
anda no buffer e escreve três `u8` — não há alocação, não há `String`, não há
`dict`.

## Carga real: `dmesg`

1397 linhas (`dmesg` do sistema), grade 100x40, melhor de 20 passadas:

| payload | Rust | Python | ganho |
|---|---|---|---|
| sem cor (109 KB) | **2,11 ms** (51,7 MB/s, 1,5 µs/linha) | 42,6 ms | 20x |
| `--color=always` (134 KB) | **2,22 ms** (60,2 MB/s, 1,6 µs/linha) | 78,6 ms | 35x |

É este par que explica a UI do Python "congelar" ao despejar log: 42–79 ms de
trabalho no thread da UI, várias vezes por segundo, contra 2,2 ms. No Rust o
`dmesg` inteiro custa menos que um frame de 60 Hz.

## Pegada

| | Rust | Python |
|---|---|---|
| binário / instalação | **10,1 MB**, um arquivo | ~6,2 mil linhas + GTK3, Pango, cairo, PyGObject, fontconfig |
| dependências de sistema | `libc`, `libm`, `libgcc_s` (+ driver gráfico em runtime) | stack GTK completo |
| startup até abrir janela e rodar comando | **0,18 s** | ~0,6–1,2 s (importa `gi`, registra Pango) |
| `cargo build --release` do zero | ~2 min (a maior parte é `wgpu`) | — |

## Protocolo (para comparar dois terminais)

1. `cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor` — anote.
2. Feche o que anima (browser, vídeo, outro terminal com `yes`).
3. `tools/benchrun.py --frames 1200` e a mesma coisa com `--app <outro>`.
4. Se o grid não bater, **não compare**: o `benchrun` sai com código 3.
5. Rode 3 vezes e compare medianas, não o melhor tiro.
6. Se for comparar com o Python, meça o mesmo `--font-size` e a mesma grade.

## Armadilhas

- **Governor**: `powersave` corta o número pela metade. Foi a "anomalia" que
  custou um dia no projeto Python.
- **Vsync**: medir sem `--no-vsync` mede o monitor. O padrão do app é vsync (é o
  certo para uso); o bench passa `--no-vsync` e diz isso na saída.
- **Grid**: sem travar por `resize` + conferir em `state`, o WM entrega outra
  grade e a comparação vira laranja com banana.
- **Gerador**: acima de ~55 MB/s você está medindo o `doomfire.py`.
- **Frequência**: `schedutil` dá 3,6–4,3 GHz conforme o aquecimento. Por isso o
  `benchrun` amostra e imprime `ghz média/pico`; tiro com metade do clock não é
  tiro.
- **Compositor**: com compositor redirecionando, o `draw` mede cópia de buffer
  também. Anote se a sessão tem compositor.
- **Primeiro frame**: o `benchrun` espera o socket e o grid antes de digitar,
  mas o primeiro segundo ainda tem shader/fonte subindo. Os 1200 frames cobrem
  isso; 100 frames não cobrem.
