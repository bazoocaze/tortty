# bzmux (Rust)

Terminal VT em Rust: pty + parser VT + grade próprios, desenho na GPU com
`wgpu`. É a continuação do bzmux Python/GTK (`~/tortty-python`). A
migração está **fechada** (2026-11-02): [`docs/MIGRATION.md`](docs/MIGRATION.md)
é o registro dela, uma linha por funcionalidade do Python com o estado final e a
referência `arquivo:linha`.

Regra de convivência entre os dois repositórios: o Python **foi** a
especificação observável enquanto a migração corria; hoje ele é histórico —
serve para responder "como o app se comportava" e para julgar se algo aqui é
regressão. O que ainda falta aqui **não é paridade**: é atividade nova, e mora
no [`docs/BACKLOG.md`](docs/BACKLOG.md). Só entra em
[`docs/LIMITATIONS.md`](docs/LIMITATIONS.md) o que for decisão de não fazer (o
que ele próprio lista); pendência de migração não existe mais.

## Documentação

Cada fato mora em **um só lugar**. Todos são escritos para leitura de agente;
só o README.md é para humano.

- `AGENTS.md` (este) — convenções, arquitetura, ferramentas.
- `TASK-bzmux-in-rust.md` — registro datado da fase 1 (o experimento de
  viabilidade): perguntas, decisões, o que foi medido naquele dia. Não é leitura
  corrente; o que continua valendo foi promovido para os docs abaixo.
- `docs/MIGRATION.md` — **o registro da migração**: inventário completo das
  funcionalidades do bzmux Python, o estado final de cada uma aqui
  (`feito`/`parcial`/`falta`/`não`) e a ordem em que ela foi feita. Fechado em
  195/195; **não é fila**. **Só escopo de migração**: ideia que o Python não tem
  não entra aqui.
- `docs/BACKLOG.md` — a fila: atividade que o Python não tem (id `B<numero>`,
  os antigos `M-*`, cada uma com gatilho e custo) e as regras da fila. Se o Python
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
- Comentário e mensagem explicam **por quê**; nenhum comentário narra o óbvio.
- Nomes de arquivo e símbolos (variáveli, campo, etc) em inglês.
- Para o que já está em português, a correção é sob demanda, durante a edição, no commit
  que já toca o arquivo.
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
