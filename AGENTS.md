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
  (`feito`/`parcial`/`falta`/`não faremos`) e a ordem de execução. Uma linha por
  funcionalidade; é por ele que se responde "quanto falta". **Só escopo de
  migração**: ideia que o Python não tem não entra aqui.
- `docs/BACKLOG.md` — o que fica fora do escopo da migração: melhoria além da
  paridade (ids `M-*`, cada uma com gatilho) e as regras da fila. Se o Python
  não faz, não é paridade: é backlog.
- `docs/BENCHMARKS.md` — **o único lugar com número** de vazão, parse, desenho
  e pegada (com data, governor e condição). Os outros docs citam o fato em uma
  frase e linkam.
- `docs/CONFIG.md` — o arquivo de configuração, chave por chave. Um teste
  cobra que toda chave de `DEFAULT_CONF` apareça lá (a doc não envelhece).
- `docs/INSTALL.md` — instalar/desinstalar o comando `bzmux` (e o que sai em
  cada prefixo). É o único lugar que descreve o instalador.
- `docs/DEBUG.md` — manual operacional: como subir/matar sem derrubar a sessão,
  sondas, o contrato do debug socket e as armadilhas (winit/wgpu/pty/parser).
  **Ler antes de rodar qualquer ferramenta gráfica.**
- `docs/LIMITATIONS.md` — o que não é feito e por quê (decisão ou limite
  aceito, não fila).
- `docs/HISTORY.md` — registro datado do que foi feito (narrativa). O histórico
  da fase Python fica no outro repo, em `docs/HISTORY.md` de lá.
- `README.md` — visão para humano: rodar, medir, estrutura.

### Regra não se cria sozinho

Fato, medição e código podem ser registrados direto. **Norma não**: antes de
acrescentar ou mudar uma regra em qualquer doc — critério de entrada de fila,
definição de estado, "o que vai aonde", formato de registro, seção nova que
alguém precisa alimentar — proponha o texto e espere o ok do dono do projeto.
Vale para este arquivo também.

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

- Versão: incrementar o patch em `Cargo.toml` (raiz, `workspace.package`) quando
  **fechar um item** de `docs/MIGRATION.md` ou `docs/BACKLOG.md` — o commit que
  fecha o item leva o incremento, não cada commit. Commit de docs, teste ou
  ajuste avulso não incrementa. É a fonte única: os dois crates herdam
  (`version.workspace = true`) e o `--version` mostra
  (`env!("CARGO_PKG_VERSION")`, compilado no build).
- Comentário e mensagem explicam **por quê**, em português; **nome de arquivo,
  símbolo, variável e campo em inglês**. Nenhum comentário narra o óbvio.
- O que já está em português (`dano.rs`, `linhas_remontadas`, `escolhe_adaptador`)
  fica como está até alguém mexer nele: a correção é **sob demanda**, no commit
  que já toca o arquivo. Renomear em massa não paga o diff e atrapalha o `git
  blame`.
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
- `./tools/visual.sh` — verificação visual **sem aparecer na tela do dono**:
  Xvfb próprio, tema fixo, captura e comparação de pixel (`tools/pixel.py`).
  Sai 0/1, então serve de gate. `./tools/pixel.py shot.png --diff a.png b.png`
  diz quantos pixels mudaram e onde.
- `./tools/bench.sh` — escada Rust x Python nas duas grades.
- `cargo run --release -p bzmux-core --bin parsebench -- --file /tmp/dmesg.txt`
  — parse de carga real, sem display.
