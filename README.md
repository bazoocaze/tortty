# bzmux em Rust — experimento de viabilidade

Terminal VT com pty, escrito em Rust: parser VT próprio, grade própria e
desenho na GPU (`wgpu`) por instâncias. É o experimento da pergunta "reescrever
o bzmux (Python/GTK) em Rust resolve distribuição e desempenho?" — o andamento,
os números e as decisões estão em [`TASK-bzmux-in-rust.md`](TASK-bzmux-in-rust.md).

Estado: **migração concluída** (2026-11-02). O experimento de viabilidade
fechou com 8–9,5x o Python em vazão ([`docs/BENCHMARKS.md`](docs/BENCHMARKS.md))
e a decisão foi migrar; o painel de [`docs/MIGRATION.md`](docs/MIGRATION.md)
fechou em **200 de 200** linhas de paridade. Está aqui, funcionando: parser VT,
grade, pty, GPU (Vulkan, GL e CPU), teclado (protocolo do kitty incluso), mouse,
seleção/clipboard, abas e workspaces, imagens do kitty, configuração em arquivo
com recarga a quente, reflow, ligaduras, emoji/CJK, busca não. O que vem agora é
atividade nova, na fila de [`docs/BACKLOG.md`](docs/BACKLOG.md).

## Documentação

| arquivo | para que serve |
|---|---|
| [`docs/MIGRATION.md`](docs/MIGRATION.md) | o registro da migração: funcionalidade, estado final, origem no Python |
| [`docs/BENCHMARKS.md`](docs/BENCHMARKS.md) | o único lugar com número (vazão, parse, desenho, pegada) |
| [`docs/CONFIG.md`](docs/CONFIG.md) | o arquivo de configuração: chaves, padrões e o que é erro |
| [`docs/INSTALL.md`](docs/INSTALL.md) | instalar, desinstalar, verificar e publicar binário |
| [`docs/DEBUG.md`](docs/DEBUG.md) | ferramentas, debug socket e armadilhas — ler antes de rodar GUI |
| [`docs/PORTABILITY.md`](docs/PORTABILITY.md) | Linux/Windows/macOS: o que funciona, o que bloqueia, distribuição |
| [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md) | o que não é feito e por quê (decisão, não fila) |
| [`docs/BACKLOG.md`](docs/BACKLOG.md) | fila de trabalho |
| [`docs/HISTORY.md`](docs/HISTORY.md) | registro datado do que foi feito |
| [`AGENTS.md`](AGENTS.md) | convenções, arquitetura e ferramentas (para agente) |
| [`TASK-bzmux-in-rust.md`](TASK-bzmux-in-rust.md) | registro datado da fase 1 (o experimento) |

## Instalar

```sh
./install.sh                 # compila e instala em ~/.local/bin/bzmux
./install.sh --check         # confere binário, libs e menu
./install.sh --uninstall     # remove comando, entrada e ícone
```

O bzmux é um binário só (libc/libm + driver da GPU). Detalhes, `--prefix`,
`--from-release` e a convivência com o bzmux Python (`tortty-python`): `docs/INSTALL.md`.

## Rodar

```sh
cargo build --release
./target/release/bzmux                          # $SHELL em 80x24
./target/release/bzmux --cmd 'mc' --cols 100 --rows 30
./target/release/bzmux --cmd 'vim arquivo.rs' --font-size 13
./target/release/bzmux --no-vsync --debug-socket auto --prof
```

`--font-size` é em **pontos** (como o `font_size` do bzmux Python); `--dpi`
(default 96) converte para pixels e multiplica pelo `scale_factor` do monitor —
é o equivalente ao `Xft.dpi` do GTK. `--debug-socket auto` abre
`$XDG_RUNTIME_DIR/bzmux-<pid>.sock`.

## Qualidade

```sh
./tools/check.sh          # fmt + clippy (-D warnings) + testes + build + parsebench
./tools/check.sh --fix    # conserta formatação e o que o clippy corrige sozinho
```

Roda também no `pre-commit` (`git config core.hooksPath .githooks`) e no CI.

Além disso, dois gates que não precisam da sua tela:

```sh
./target/release/bzmux --selftest   # 31 checks sem janela (paridade com o checks.py)
./tools/visual.sh                   # Xvfb próprio: desenha, captura e compara pixels
```

## Medir

```sh
cargo run --release -p bzmux-core --bin parsebench        # parser isolado
./tools/bench.sh                                          # escada Rust x Python
./tools/benchrun.py --frames 1200                         # uma medição do doom-fire
```

O gerador é o `tools/doomfire.py` do tortty-python, copiado sem alteração, para os
dois terminais consumirem exatamente os mesmos bytes.

## Ferramentas de dev (PID file, nunca `pkill`)

```sh
./tools/devrun.sh --cmd 'mc' --debug-socket /tmp/bz.sock   # sobe em background
./tools/devkill.sh                                          # mata SÓ esse PID
```

`pkill -f bzmux` mata o shell que chamou (o padrão está na linha de comando) e
`pkill -x bzmux` mata o `bzmux` do pipx, que tem o mesmo nome do binário. Já
aconteceu; por isso aqui só se mata PID registrado, conferindo `/proc/<pid>/exe`.

## Estrutura

```
crates/bzmux-core/         emulação pura, sem GUI/pty/GPU (testável sem display)
  src/{cell,color,grid,charset,term}.rs
  src/bin/parsebench.rs    benchmark do parser: doomfire e arquivo (dmesg)
crates/bzmux/              app
  src/{main,cli,app,pty,keys,glyph,atlas,render,debugsock,prof}.rs
  src/shader.wgsl          um pipeline para fundo, glifo, sublinhado e cursor
tools/{doomfire,benchrun}.py, bench.sh, devrun.sh, devkill.sh
```

A regra herdada do projeto Python: `bzmux-core` não conhece janela, pty nem
GPU. Por isso `cargo test -p bzmux-core` roda sem display e o `parsebench` mede
o parser, não o toolkit.

## Números de hoje (ver o TASK)

- parser: 0,18–0,20 µs/célula no frame do doom-fire (Python pós-P20: 5,49).
- `dmesg` (1397 linhas, 100x40): 2,1 ms sem cor / 2,2 ms com cor (Python:
  42,6 / 78,6 ms).
- ponta a ponta, 61x18, 1200 frames, mesmo display: 917–968 fps / 39–42 MB/s
  contra 113–121 fps / 4,9–5,2 MB/s do Python.
