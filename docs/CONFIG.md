# Configuração

Um arquivo, formato do kitty: **uma opção por linha**, `chave valor`, espaço nas
pontas ignorado e `#` comentando até o fim da linha (desde que esteja no começo
da linha ou depois de espaço — é o que deixa `foreground #e6e6e6` ser cor e não
comentário). Chave repetida: a última vence.

```sh
~/.config/bzmux/bzmux.conf          # ou $XDG_CONFIG_HOME/bzmux/bzmux.conf
```

Não precisa criar: **os padrões estão embutidos** em
`crates/bzmux-core/src/config.rs` (`DEFAULT_CONF`) e o seu arquivo entra por
cima, chave a chave. Um teste garante que os padrões parseiam limpos e que toda
chave deles aparece nesta página — a documentação não envelhece em silêncio.

As opções de linha de comando (`--font-size`, `--scrollback`, `--cmd`…)
sobrescrevem o arquivo.

## Chaves

| chave | padrão | o que faz |
|---|---|---|
| `shell` | vazio | programa inicial das abas; vazio = `$SHELL` (o `--cmd` vence) |
| `font_family` | `Monospace` | família da fonte; só monoespaçada (a célula é o avanço do "M") |
| `font_size` | `12` | tamanho em **pontos** (4..96); o DPI converte para pixel |
| `cursor_shape` | `block` | `block`, `beam` ou `underline`; `CSI 6 q` de um app vence enquanto durar |
| `cursor_blink` | `off` | cursor piscando (500 ms por fase); tecla ou colagem deixa o cursor aceso na hora. `CSI 6 q` com forma par (4/6) pede cursor fixo e vence enquanto o programa quiser |
| `scrollback` | `10000` | linhas de histórico acima da tela (teto 100000) |
| `window_padding_width` | `0` | margem em px em volta da grade (teto 200) |
| `bell` | `none` | BEL: `none`, `visual` (marca a janela) ou `audible` |
| `copy_on_select` | `off` | seleção já vai para CLIPBOARD (PRIMARY é sempre) |
| `debug_log` | `off` | log de diagnóstico; `off` desliga, caminho liga |
| `foreground` | `#d6d6d6` | cor do texto (`#rrggbb` ou `#rgb`) |
| `background` | `#000000` | cor do fundo |
| `cursor_color` | `#d6d6d6` | cor do cursor |
| `color0` … `color15` | ver arquivo | paleta ANSI; o resto da paleta 256 (cubo e cinzas) é calculado |

O padrão das cores é a **paleta clássica do xterm** (fundo preto, azul
`#0000ee`), a mesma do bzmux Python. Não é preguiça: `mc`, `htop`, `ls
--color` e `git diff` foram calibrados nela, e trocar por um tema "bonito"
muda o significado das cores que o programa escolheu — o `mc` azul vira outra
coisa.

Booleano aceita `on/yes/true/1` e `off/no/false/0`.

## Erro é por linha

- **carga inicial**: o que validou vale; a chave inválida fica no padrão e o
  erro sai com o número da linha (`linha 7: 'font_size': "abc" não é número`);
- **reload** (editar o arquivo com o app aberto): **qualquer** erro mantém a
  configuração que já estava valendo. Meia configuração é pior que nenhuma.

Chave desconhecida é erro, não é ignorada em silêncio.

## Cores mudam em tempo de execução

Um programa pode consultar (`OSC 10;?`, `OSC 11;?`, `OSC 4;N;?`) e **alterar**
(`OSC 4;N;cor`, `OSC 10`, `OSC 11`, `OSC 12`, e os resets `104`/`110`/`111`/`112`)
as cores. A alteração vale para a sessão e vai direto para o renderer; o
arquivo de configuração só é relido quando você o edita (e aí ele vence: as
cores do arquivo substituem o que o programa tinha pedido).

## Ainda não

Paleta como tema completo (16 cores + as 240 do cubo), remapeamento de atalhos
e `include` de outro arquivo estão na fila (`docs/MIGRATION.md`, itens K20/K21 e
a melhoria M-H).
