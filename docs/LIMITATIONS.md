# Limites

O que o bzmux **não** faz e por quê. Aqui só entra **decisão** ou consequência
aceita do escopo. Funcionalidade do bzmux Python que ainda não foi migrada é
`falta` em [`MIGRATION.md`](MIGRATION.md) — não é limite. Se um limite deixar de
existir, sai daqui e vira item concluído em [`HISTORY.md`](HISTORY.md).

## Escopo

- **Um usuário, uma máquina.** Sem multiplexação de sessão, sem servidor, sem
  reconexão, sem daemon. Fechar a janela termina tudo.
- **Um terminal, não um ambiente de janelas.** Sem splits nesta fase
  (`MIGRATION.md` M-J), sem abas persistentes até a F4, sem gerenciador de
  layout.
- **Só o protocolo de imagem do kitty.** Sem sixel, sem iTerm2 inline, sem
  `t=f`/`t=t`/`t=s` (arquivo, temp, memória compartilhada) e sem animação
  (`a=f`/`a=a`). A imagem tem que vir no escape base64.
- **`TERM=xterm-256color`.** Não inventamos entrada de terminfo própria: o
  objetivo é que `vim`, `mc`, `less` e `dmesg` funcionem com o que já existe.

## Plataforma

- **Linux X11 é o alvo testado.** Wayland, Windows e macOS estão na rota
  ([`PORTABILITY.md`](PORTABILITY.md)) mas **não foram exercitados**: até
  passarem por CI, "funciona lá" é expectativa, não fato.
- **Posição da janela no Wayland**: o cliente não escolhe onde a janela fica.
  Quando a geometria for persistida (J2/J3), no Wayland só tamanho e maximizado
  são guardados. Não é falta de código, é o protocolo.
- **Sem backend de software próprio.** Se não houver Vulkan/Metal/D3D12/GL
  utilizável, o app sai com mensagem. Rasterizar por CPU é um renderer inteiro.
- **Sem assinatura/notarização por enquanto.** macOS 15+ exige "Abrir Mesmo
  Assim" para binário baixado pela rede. A decisão sobre a conta de developer
  (US$ 99/ano) está em aberto em [`PORTABILITY.md`](PORTABILITY.md).

## Emulação

- **Wide char funciona, é melhoria consciente** — e por isso o modelo de célula
  tem marca de continuação (`WIDE_CONT`). O bzmux Python não tem: lá CJK ocupa
  uma célula. Toda operação nova de grade precisa respeitar o par.
- **Combinação de grafema**: cada célula carrega um `char`; marca combinante
  que não veio junto no mesmo commit do texto vira célula própria. Compor de
  verdade é melhoria (`MIGRATION.md` A12), não limite aceito para sempre.
- **Sem ligaduras/shaping complexo** (G8): `swash` rasteriza glifo, não faz
  shaping de HarfBuzz. Não há plano de trocar por `harfbuzz` nesta fase.
- **Sem `IRM`/`LNM`** (insert mode, newline mode) e sem `CSI ? 3` (80/132):
  raros a ponto de não pagar o teste. Ficam registrados no parser como
  pendência conhecida.
- **Sem sixel/DECDL/DECDHL** e sem double-height: nem o Python tem.

## Interface

- **Atalhos fixos até a F3.** Não há remapeamento; a config em arquivo (que
  vem na F3) cobre fonte, cores, padding, bell, scrollback.
- **Sem tema escuro/claro automático**: a paleta vem da config, não do portal
  do desktop.
- **Sem menu de contexto e sem DnD** de arquivo na janela.

## Desenvolvimento

- **Número de desempenho mora em um lugar** ([`BENCHMARKS.md`](BENCHMARKS.md)) e
  não em README, TASK ou comentário de código. Repetir número produz divergência
  e cada re-medição vira uma caçada.
- **`--no-vsync` não é modo de uso**: é instrumento. Em uso normal o desenho é
  limitado pelo vsync, e é assim que deve ser.
