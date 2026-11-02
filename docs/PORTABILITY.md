# Portabilidade (Linux, Windows, macOS)

Substitui o estudo `P30` do repo Python (`docs/PORTABILITY.md` de lá), que
avaliava rotas **dentro** do Python (conda, Qt/PySide6, FFI). Aqui a decisão já
foi tomada e o que resta é engenharia: a rota é **`winit` + `wgpu` + núcleo
próprio em Rust**, sem toolkit de sistema, sem Pango/cairo/fontconfig e sem
Python. Ver [`MIGRATION.md`](MIGRATION.md) para o que falta de funcionalidade.

## Por que esta rota responde à pergunta original

O pedido que abriu o experimento: "distribuir entre win/lin/mac é difícil no
Python". O bloqueio não era o Python em si, era o **stack de sistema**: GTK3 +
Pango + cairo + PyGObject + fontconfig, cada um com um jeito diferente de instalar
em cada plataforma. Nesta rota esse stack não existe:

| | Python/GTK | Rust/wgpu |
|---|---|---|
| binário | não gera binário (precisa de Python + libs) | **10,1 MB, um arquivo** |
| deps do SO | GTK3, Pango, cairo, PyGObject, fontconfig | `libc`, `libm`, `libgcc_s` (+ driver gráfico em runtime) |
| fonte | Pango + fontconfig | `fontdb` + `swash` (varre o sistema, sem fontconfig) |
| GPU | cairo (glamor/software) | Vulkan / Metal / D3D12 / GL pelo `wgpu` |
| janela | GTK3 | `winit` (X11, Wayland, Win32, Cocoa) |
| pty | `pty.fork` (POSIX só) | `portable-pty` (pty POSIX **e** ConPTY) |

Consequência prática: **"copiar o executável e rodar"** funciona dentro do mesmo
sistema e arquitetura. Não funciona entre sistemas — binário Linux não roda em
macOS nem em Windows; é preciso **compilar** para cada alvo (e é isso que a
matriz de CI abaixo faz de graça).

## Estado por plataforma

| alvo | render | pty | testado? |
|---|---|---|---|
| Linux X11 + Vulkan (RADV) | ● | ● | **sim**, é o ambiente de desenvolvimento |
| Linux Wayland | deve funcionar (`winit` nativo) | ● | **não** — falta exercitar |
| Linux + GL (`WGPU_BACKEND=gl`) | deve funcionar | ● | **não** — válvula para GPU velha |
| Linux + llvmpipe (software) | deve funcionar, lento | ● | **não** — próximo teste de portabilidade |
| Windows 10/11 | D3D12 (ou Vulkan) | ConPTY | **não** — precisa de runner Windows |
| macOS Apple Silicon | Metal | POSIX pty | **não** — precisa de runner macOS |
| macOS Intel | Metal | POSIX pty | **não** — runner `macos-13` |

## Bloqueios concretos no código de hoje

Ordenados por gravidade. Nenhum é arquitetural; todos são de fronteira.

1. **Socket de debug é Unix socket** (`UnixListener`). No Windows não existe:
   precisa de named pipe ou TCP em `127.0.0.1`. Decisão: manter Unix socket nos
   sistemas POSIX e usar named pipe (`\\.\pipe\bzmux-<pid>`) no Windows, com a
   mesma moldura JSON por linha. O `tools/benchrun.py` passa a escolher o
   transporte pelo caminho.
2. **`$XDG_RUNTIME_DIR`** para `--debug-socket auto`: no Windows é
   `%LOCALAPPDATA%\Temp`, no macOS é `$TMPDIR`. Uma função `runtime_dir()` e
   acabou.
3. **`$SHELL` e `/bin/sh`** como default: no Windows o shell é `$COMSPEC`
   (`cmd.exe`) e o `--cmd` precisa ser `cmd /c`. O `portable-pty` já abstrai o
   spawn; falta a escolha do shell por plataforma.
4. **Varredura de fonte**: `fontdb` cobre os diretórios de fonte de cada sistema,
   mas a **lista de preferência** hoje é de fonte Linux (DejaVu, Liberation,
   Noto). No macOS a monoespaçada padrão é Menlo/SF Mono; no Windows, Consolas e
   Cascadia Mono. É uma lista por `cfg(target_os)`.
5. **Caminho de configuração** (`$XDG_CONFIG_HOME`): no Windows
   `%APPDATA%\bzmux`, no macOS `~/Library/Application Support/bzmux`. Só existe
   quando a config em arquivo for migrada (ver MIGRATION).
6. **Clipboard e IME**: quando entrarem, são plataforma-específicos
   (`arboard`/`winit::Ime`); o `winit` já expõe IME em todos os alvos.
7. **Bracketed paste / terminal modes**: nada a fazer, é do parser.
8. **Wayland**: o cliente não escolhe posição de janela (nunca vai escolher).
   Persistência de geometria no Wayland guarda tamanho e maximizado, não `x`/`y`.
   Isso já era limite aceito no Python (`LIMITATIONS.md` de lá).

## Distribuição

Caminho recomendado, nesta ordem:

1. **GitHub Actions com matriz** (`ubuntu-latest`, `windows-latest`,
   `macos-14` arm64, `macos-13` x86_64) gerando artefatos por tag. Em repo
   público é grátis; em privado, minuto de macOS conta 10x.
2. **`lipo`** para universal binary no macOS (arm64 + x86_64) — um `.app`
   simples ou binário solto + `.dmg`/`.zip`.
3. **Assinatura**: no Apple Silicon todo executável precisa de assinatura, mas o
   linker já aplica *ad-hoc signature* em arm64 — o que sai do `cargo build` roda.
   O que **não** roda sem intervenção é binário baixado pela rede não notarizado
   (Gatekeeper): o usuário precisa de "Abrir Mesmo Assim". Para evitar esse atrito
   há duas saídas, e **nenhuma é uma "loja"**:
   - **Apple Developer Program** (US$ 99/ano) → notarização → duplo clique abre
     sem aviso. É o único jeito "limpo".
   - **instalação que contorna a quarentena**: script `curl | sh` que compila do
     fonte, ou um *tap* do Homebrew que faz build. Aí não há `com.apple.quarantine`
     no arquivo e o Gatekeeper não entra. Menos confortável, custo zero.
4. **Windows**: `.zip` com o `.exe` funciona direto (SmartScreen reclama de
   assinatura ausente — `signtool` com certificado OV/EV resolve, se valer).
   ConPTY é nativo desde o Windows 10 1809.
5. **Linux**: binário estático (`musl`) por arquitetura, `.deb`/`.rpm`/AUR depois.
   Não é prioridade: no Linux o usuário compila.

Custo do Windows/macOS sem loja: **um `.zip` no GitHub Releases**. O que dói é
notarização (macOS) e assinatura (Windows), ambos opcionais para uso próprio e
para quem usa `curl | sh`.

## Decisão

Rota única, sem bifurcação: `winit` + `wgpu` + núcleo próprio. Linux X11 é o
alvo de desenvolvimento e o único testado; os outros dois entram por CI quando a
migração de funcionalidades estiver perto do fim (não antes: porta sem feature é
retrabalho). O que **não** se faz: cross-compile de macOS a partir do Linux
(`osxcross` exige SDK da Apple, é frágil e não passa em revisão), nem manter o
stack GTK como alternativa.
