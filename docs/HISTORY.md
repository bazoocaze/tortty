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
