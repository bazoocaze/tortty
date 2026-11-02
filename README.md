# bzmux em Rust — experimento de viabilidade

Terminal VT com pty, escrito em Rust: parser VT próprio, grade própria e
desenho na GPU (`wgpu`) por instâncias. É o experimento da pergunta "reescrever
o bzmux (Python/GTK) em Rust resolve distribuição e desempenho?" — o andamento,
os números e as decisões estão em [`TASK-bzmux-in-rust.md`](TASK-bzmux-in-rust.md).

Estado: **experimento funcional**. Abre janela, roda shell em pty, desenha a
grade, funciona em `vim`, `mc` e despejo de console (`dmesg`), e tem o mesmo
debug socket JSON do bzmux Python. Fora de escopo ainda: abas/workspaces,
imagens do kitty, config em arquivo, seleção/clipboard, mouse reporting, reflow.

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
