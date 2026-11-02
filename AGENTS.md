# bzmux (Rust)

Terminal VT em Rust: pty + parser VT + grade próprios, desenho na GPU com
`wgpu`. É a continuação do bzmux Python/GTK (`~/tortty-python`), que
fica no papel de **referência de comportamento** até a migração terminar:
[`docs/MIGRATION.md`](docs/MIGRATION.md) é o checklist do que falta, item por
item, com o estado de cada um.

Regra de convivência entre os dois repositórios: o Python é a **especificação
observável** (o que o app faz hoje), este é o destino. Funcionalidade do Python
que ainda não existe aqui é **pendência** (`docs/MIGRATION.md`), não limite. Só
entra em [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md) o que for decisão de não
fazer — hoje: wide char (que aqui funciona e lá não, de propósito) e o que o
próprio `LIMITATIONS.md` lista.

## Documentação

Cada fato mora em **um só lugar**. Todos são escritos para leitura de agente;
só o README.md é para humano.

- `AGENTS.md` (este) — convenções, arquitetura, ferramentas.
- `TASK-bzmux-in-rust.md` — registro datado da fase 1 (o experimento de
  viabilidade): perguntas, decisões, o que foi medido naquele dia. Não é leitura
  corrente; o que continua valendo foi promovido para os docs abaixo.
- `docs/MIGRATION.md` — **o painel de controle da migração**: inventário
  completo de funcionalidades do bzmux Python, o estado de cada uma aqui
  (`feito`/`parcial`/`falta`/`não faremos`), a ordem de execução e as melhorias
  propostas. Uma linha por funcionalidade; é por ele que se responde "quanto
  falta".
- `docs/BENCHMARKS.md` — **o único lugar com número** de vazão, parse, desenho
  e pegada (com data, governor e condição). Os outros docs citam o fato em uma
  frase e linkam.
- `docs/INSTALL.md` — instalar/desinstalar o comando `bzmux` (e o que sai em
  cada prefixo). É o único lugar que descreve o instalador.
- `docs/DEBUG.md` — manual operacional: como subir/matar sem derrubar a sessão,
  sondas, o contrato do debug socket e as armadilhas (winit/wgpu/pty/parser).
  **Ler antes de rodar qualquer ferramenta gráfica.**
- `docs/LIMITATIONS.md` — o que não é feito e por quê (decisão ou limite
  aceito, não fila).
- `docs/BACKLOG.md` — fila de trabalho e as regras da fila.
- `docs/HISTORY.md` — registro datado do que foi feito (narrativa). O histórico
  da fase Python fica no outro repo, em `docs/HISTORY.md` de lá.
- `README.md` — visão para humano: rodar, medir, estrutura.

## Arquitetura

Workspace de dois crates. A fronteira é rígida e é a razão de o núcleo ser
testável sem display:

- `crates/bzmux-core` — emulação pura: célula, paleta, charset DEC, grade,
  parser VT, scrollback. **Não conhece janela, pty, GPU, relógio, thread nem
  configuração.** Nenhum `wgpu`, nenhum `winit`, nenhuma dependência de sistema.
  Se precisar de algo do mundo, o chamador passa por parâmetro ou lê o resultado
  de um evento devolvido pelo `feed`.
- `crates/bzmux` — o app: `pty` (portable-pty), `app` (winit + laço de
  eventos), `render`/`glyph`/`atlas` (wgpu + swash), `keys`, `debugsock`,
  `prof`, `cli`.
- `tools/` — `devrun.sh`/`devkill.sh` (PID file), `benchrun.py`, `bench.sh`,
  `doomfire.py` (byte a byte igual ao do repo Python: é a mesma carga).

Fluxo de um byte: thread leitora do pty → `Term::feed` (core) → marca dano na
linha → `request_redraw` → `RedrawRequested` monta o vetor de instâncias →
`wgpu` desenha (fundo, glifo, sublinhado, cursor: um pipeline, uma draw call
por categoria, na ordem).

## Convenções de código

- Comentário e mensagem explicam **por quê**, em português; nome de código em
  inglês. Nenhum comentário narra o óbvio.
- Nada de `unsafe` no core. No app, só onde a API do `wgpu`/`winit` exige.
- Medição antes de otimizar: `parsebench` para o parser, `tools/benchrun.py`
  para o app. Mudança que mexe no caminho quente vem com o número antes e
  depois no mesmo commit.
- Toda sequência implementada no parser entra com teste em `bzmux-core`
  (tabela `#[test]` no fim do módulo, nome do teste = o que ele garante).

## Qualidade: um comando só

```sh
./tools/check.sh          # fmt --check + clippy -D warnings + teste + build
./tools/check.sh --fast   # sem o build (é o pre-commit)
./tools/check.sh --fix    # aplica `cargo fmt` e `clippy --fix` antes de checar
```

O **mesmo** script roda no git hook (`.githooks/pre-commit`, ativado com
`git config core.hooksPath .githooks`) e no CI (`.github/workflows/ci.yml`).
Hook e CI que discordam não servem para nada.

- **`cargo fmt`** é a formatação: `rustfmt.toml`, largura 100, só opções
  estáveis. Não discuta estilo no review — `cargo fmt` decide.
- **`cargo clippy`** roda com `-D warnings` e liga `all`, `correctness`,
  `suspicious` e `pedantic`. Os lints desligados estão em `Cargo.toml` com o
  motivo escrito (cast de célula/pixel, `unsafe_code = deny` com allow local).
- **`allow` de lint é local e comentado**, nunca no topo do crate: se a regra
  não vale para uma função, o `#[allow]` mora nela e diz por quê.
- Commit só passa com a régua verde.

## Ferramentas

- `./tools/devrun.sh --cmd 'mc' --debug-socket /tmp/bz.sock` — sobe em
  background com PID file.
- `./tools/devkill.sh` — mata **só** o PID registrado, conferindo
  `/proc/<pid>/exe`. `pkill -f bzmux` mata o shell que chamou (o padrão está na
  linha de comando) e `pkill -x bzmux` mata o `bzmux` do pipx, que tem o mesmo
  nome do binário. Já aconteceu; não use.
- `./tools/bench.sh` — escada Rust x Python nas duas grades.
- `cargo run --release -p bzmux-core --bin parsebench -- --file /tmp/dmesg.txt`
  — parse de carga real, sem display.
