# Instalar

O bzmux é **um binário**. Não há passo de dependência de sistema: o `cargo
build --release` liga só em `libc`/`libm`/`libgcc_s`, e o que sobra em tempo de
execução é o driver da GPU (Vulkan/GL) e uma fonte mono do sistema. O bzmux
Python precisava de `python3-gi`, `gir1.2-gtk-3.0`, cairo, Pango e pipx; aqui
nada disso.

## Comando

```sh
./install.sh                 # compila (release) e instala em ~/.local/bin/bzmux
./install.sh --prefix DIR    # outro prefixo: o comando fica em DIR/bin/bzmux
./install.sh --binary ARQ    # instala um binário já compilado (não compila)
./install.sh --from-release  # baixa o binário do GitHub Releases (sem cargo)
./install.sh --local         # comando vira link para target/release/bzmux (dev)
./install.sh --no-shortcut   # sem entrada no menu do sistema
./install.sh --check         # confere a instalação existente
./install.sh --uninstall     # remove comando, entrada e ícone
./install.sh --dry-run       # mostra cada comando, não executa
```

O que ele faz, em ordem:

1. obtém o binário (compila, copia `--binary` ou baixa `--from-release`);
2. avisa se `bzmux` já existe no `PATH` e não é o desta instalação;
3. instala em `$PREFIX/bin/bzmux` (`install`, copiando — trocar o binário no
   `target/` não deixa a instalação apontando para o nada);
4. sem `--no-shortcut`, grava `$PREFIX/share/applications/bzmux.desktop` e
   `$PREFIX/share/icons/hicolor/scalable/apps/bzmux.svg`;
5. verifica: roda `bzmux --version` e falha se `ldd` mostrar biblioteca
   faltando;
6. avisa se `$PREFIX/bin` está fora do `PATH`.

`sudo` só entra quando o prefixo não é gravável (ex.: `--prefix /usr/local`).

A entrada do menu leva `StartupWMClass=bzmux` — o winit põe `WM_CLASS` igual ao
nome do binário (`"bzmux", "bzmux"`), então o agrupador de janelas do ambiente
associa o atalho à janela certa.

## Conviver com o bzmux Python

O bzmux Python instala pelo pipx com o nome `bzmux`. Com o Python passando a
instalar como `tortty-python`, o nome curto fica livre para o Rust:

```sh
pipx uninstall bzmux          # remove o shim antigo (a venv some junto)
./install.sh                  # agora `bzmux` é o Rust
tortty-python                       # o Python, com o nome novo
```

Enquanto o Python ainda instalar como `bzmux`, o instalador avisa quando
encontra o shim do pipx no caminho (o shim em `~/.local/bin` não tem "pipx" no
nome; o alvo tem: `~/.local/share/pipx/venvs/bzmux/bin/bzmux`).

Quem manda em qual nome: o `tortty-python` instala e remove **só**
`tortty-python.desktop`/`tortty-python.svg`; `bzmux.desktop`/`bzmux.svg` são desta
instalação. Verificado com `tortty-python --install-shortcut` e `--remove-shortcut`
sobre os arquivos reais (md5 igual antes e depois).

Versões do `tortty-python` anteriores a 2026-11-02 tratavam `bzmux` como "nome antigo"
e apagavam a entrada daqui no `install()`; se a entrada do menu sumir depois de
rodar aquele instalador, atualize o `tortty-python` (o caso tem regressão em
`bzmux/checks.py`).

## Verificar

```sh
./install.sh --check
# binário:  /home/USER/.local/bin/bzmux
# versão:   bzmux 0.1.0
# link:     /home/USER/.local/bin/bzmux
# libs:     5 entradas, faltando: 0
# menu:     .../applications/bzmux.desktop

bzmux --version
bzmux --cmd 'stty size' --cols 100 --rows 30   # tem que dizer 30 100
```

`--cols/--rows` são em células e o grid final é o que a janela conseguiu: se o
WM entregar menos pixels, o bzmux ajusta a janela até bater (`docs/DEBUG.md`).
O filho nasce depois desse ajuste, então `stty size` mostra o tamanho real.

## Publicar (ainda não feito)

`--from-release` espera, no GitHub Releases da tag `v<versão>`:

```
bzmux-<versão>-x86_64-unknown-linux-gnu.tar.gz    (contém o binário `bzmux`)
bzmux-<versão>-aarch64-unknown-linux-gnu.tar.gz
```

O repositório sai de `BZMUX_REPO` (default: o remote declarado neste projeto).
Falta a matriz de CI que gera esses tarballs por tag — é o item **M8** de
`docs/MIGRATION.md`; macOS/Windows dependem de assinatura e estão em
`docs/PORTABILITY.md`.
