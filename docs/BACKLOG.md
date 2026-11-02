# Backlog — atividades novas, fora da paridade

A regra é uma só, e é de **natureza**, não de gosto:

| o Python faz? | onde mora | o que é |
|---|---|---|
| **faz** | [`MIGRATION.md`](MIGRATION.md) | paridade: linha com `feito`/`parcial`/`falta`/`não` e o `arquivo:linha` de referência |
| **não faz** | **este arquivo** | atividade nova, com id `M-*`, um gatilho e um custo |

Um trabalho tem **um dono**. Item que o Python faz nunca aparece aqui; item que
ele não faz nunca aparece lá. Foi a falta dessa regra que encheu os dois
documentos com o mesmo trabalho em 2026-11-02 (ver `HISTORY.md`).

## Regras da fila (as mesmas do bzmux Python)

- **Registrar é só registrar.** A investigação (ler código, medir, reproduzir)
  acontece quando o item for executado, não quando é anotado. Na anotação vai o
  sintoma, o gatilho e o que vier de graça no caminho; nada de diagnóstico
  completo antes da hora.
- **Cada item tem um gatilho.** Melhoria sem gatilho vira desejo: só entra na
  tabela o que tem uma frase do tipo "quando X aparecer". Item sem gatilho é
  limitação ([`LIMITATIONS.md`](LIMITATIONS.md)), não fila.
- **Ao fechar um item**: sai da tabela de aberto, entra em **Fechados** aqui e
  ganha uma linha datada em [`HISTORY.md`](HISTORY.md) com o que foi verificado.
- **Bug corrigido fora de item** entra na lista numerada do `HISTORY.md`; se for
  armadilha de ambiente/ferramenta, entra em [`DEBUG.md`](DEBUG.md).
- **Decisão de não fazer** vai para [`LIMITATIONS.md`](LIMITATIONS.md).

## Aberto

| id | atividade | por que | gatilho | custo |
|---|---|---|---|---|
| M-Q | **Overlay de cromo** — barra de abas, cartões da sidebar e toast viram uma lista de elementos (retângulo + texto + área de clique) desenhada sobre a grade, no mesmo `wgpu` | hoje cada elemento de cromo é escrito **duas vezes**: o desenho em `render.rs` (`tabbar`/`sidebar`/`toast`) e o teste de clique em `app.rs` (`tab_at`/`card_at`), com as mesmas coordenadas em fórmulas separadas. Três bugs de uma sessão foram essa divergência: aba errada porque `tab_at` descontava a sidebar, cartão começando na linha errada, duplo clique no cartão da linha 1 virando clique de aba. Um `Vec<Elemento>` com hit-test declarado mata a duplicação sem toolkit e sem dependência de sistema (a grade continua célula a célula; só o cromo vira cena) | quando aparecer o **4º** elemento de cromo — e ele já tem nome: `M-V`, menu de contexto, que é justamente o gatilho do `M-Q` | baixo-médio: ~200 linhas (elemento, layout em célula, hit-test em ordem inversa de z) + migrar os 3 elementos existentes e apagar `tab_at`/`card_at`/`bar_drag` na forma atual |
| M-V | Menu de contexto (botão direito na grade): copiar, colar, abrir link, nova aba | o Python não tem; hoje o botão direito ou vira reporte de mouse para o app ou não faz nada | primeiro uso diário em que copiar exigir arrasto ou atalho | médio: depende do `M-Q` (o menu é o 4º cromo) + integração com `arboard` |
| M-C | OSC 8 hyperlink: guardar por célula, sublinhar e abrir com Ctrl+clique | links em `dmesg`, compiladores e `ls --hyperlink`; hoje o escape é consumido e descartado | quando um link de log precisar ser aberto à mão duas vezes | baixo: um `u32` por célula + hover |
| M-R | Escapes que hoje são descartados: OSC 52 (clipboard remoto), OSC 7 (cwd do shell), OSC 133 (marcas de prompt) | três escapes que o Python ignora; OSC 7/133 destravam "nova aba no cwd certo" e navegação por prompt; OSC 52 é o que faz `ssh` remoto copiar | quando `ssh` precisar colar/copiar do terminal remoto (OSC 52) | baixo: já há um handler de OSC com slot por índice |
| M-D | Busca no scrollback com realce | hoje só se acha rolando; em log grande é a função que falta | primeiro log de build que precise ser garimpado | médio: índice por linha ou varredura sob demanda |
| M-E | Seleção por palavra/linha (duplo/triplo clique) e retangular | copiar um caminho de log exige arrasto preciso; seleção em bloco é o que `ls -l` e tabela de log pedem | quando copiar caminho por arrasto incomodar duas vezes | baixo |
| M-F | IME e composição (CJK, compose) | quem digita CJK não consegue usar; o Python também não tem | primeiro usuário que digitar CJK | médio: `winit::Ime` + pré-edição na view |
| M-G | Kitty keyboard: **só a flag 4** (alternate keys) | o resto do protocolo é paridade (o Python tem `kitty_kbd.py`); a flag 4 é a única parte que ele não entrega | quando um app pedir `CSI > u` com flag 4 e receber resposta incompleta | médio |
| M-H | Config v2: remapeamento de atalhos (a paleta já está feita: `color0..15`, fg/bg/cursor) | atalho é a primeira coisa que alguém quer mudar depois da fonte, e hoje é `match` no código | quando um atalho do WM colidir com um do terminal | baixo-médio: parser pronto, falta a tabela de ações |
| M-I | Abas/workspaces persistentes entre execuções | hoje fechar a janela custa o layout; o `state` do socket já é JSON | quando fechar a janela custar o layout de um dia | baixo |
| M-J | Splits (mais de um terminal na mesma view) | pedido registrado no P39 do Python; aqui cada split é uma view no mesmo `wgpu` | quando dois terminais lado a lado forem rotina | alto |
| M-K | Notificação OSC 9/777 + urgency | "terminou o build" sem precisar olhar a janela | quando build longo exigir vigia | baixo |
| M-L | Imagens: z-index, placement id, placeholder Unicode, recorte de origem | o Python mostra imagem sempre por cima e sem controle fino | depois de a imagem básica (F6) estar funcionando com `icat` | médio |
| M-M | Barra de rolagem visível e arrastável | hoje só a roda e `Shift+Pg` (que já funcionam); falta o indicador de posição | quando rolar log grande só pela roda cansar | baixo: retângulo + estado |
| M-N | Transporte de socket no Windows (named pipe) | o `benchrun.py` e a automação precisam funcionar lá também | junto com F8 (portas) | baixo |
| M-P | Remapear a âncora da seleção no reflow | hoje o reflow mata a seleção em vez de deslocá-la (o Python também mata). O `reflow` já calcula `(parágrafo, offset)` de cada linha velha, então a âncora nova é uma conta | quando perder uma seleção longa num resize incomodar | baixo |
| M-S | Teclas que o Python não entrega: F13-F24 e teclado numérico com NumLock off (`KP_*`) | teclado estendido e numérico de notebook ficam mudos | quando alguém mapear F13 ou usar `KP_*` num app | baixo: tabela em `keys.rs` |
| M-T | Mouse reporting em pixels (`?1016`) | app gráfico pedindo coordenada de pixel recebe célula | quando um app de TUI gráfica pedir `?1016` | baixo: já há `cell_at`/`px` |
| M-U | Parser: DECALN (`ESC # 8`) e caracteres combinantes (compor na célula anterior) | DECALN é a tela de teste do VT100; combinante hoje ocupa célula própria, e é o que quebra acento morto em app que manda `U+0301` separado | primeiro acento decomposto saindo deslocado (ou teste de tela VT100) | baixo para DECALN; médio para combinante (mexe em `wcwidth`/avanço) |

## Fechados

Atividade nova que já está no programa. Não voltam para o `MIGRATION.md`: não
eram paridade quando começaram e não são paridade agora.

| id | atividade | o que ficou |
|---|---|---|
| M-B | SGR completo: itálico, faint, blink, riscado, overline, cor e estilos de sublinhado | `cell.rs`/`term.rs`/`render.rs`; verificado com `bat`, `git diff` e `mc` |
| M-O | `--selftest` no binário | 14 checagens sem display/GPU, dentro do `check.sh` e do CI |
| — | Charset G1 + SO/SI (lock shift) | `term.rs` |
| — | Wide char (`wcwidth`) com célula de continuação | `cell.rs`/`term.rs`; o Python não tem |
| — | Tab stops configuráveis (HTS/TBC/CHT/CBT) | `term.rs` |
| — | Origin mode (`?6`) e autowrap desligável (`?7`) | `term.rs` |
| — | DCS/APC/PM/SOS consumidos até ST (não vazam na tela) | `term.rs`; no Python vazam |
| — | DECSC salva SGR/origin/autowrap (não só o cursor) | `term.rs` |
| — | `ESC D` (IND) e `ESC E` (NEL) | `term.rs`; no Python eram no-op |
| — | Clamp de `n` em IL/DL/ICH/DCH/ECH | `term.rs` |
| — | SU/SD, REP (`CSI b`), teto de comprimento em `csi`/`osc` | `term.rs` |
| — | OSC 4/104/110/111/112 como **set** (redefinir cor em runtime) | `Term::theme` |
| — | Paleta/tema configurável e `color0..15`/fg/bg/cursor no arquivo | `config.rs`, `docs/CONFIG.md` |
| — | Roda com delta fracionário (trackpad) acumulando | `app.rs` |
| — | `?1005` coordenada em UTF-8 (acima do teto de 222) | `mouse.rs` |
| — | Grid travado pelo socket sobrevive ao arredondamento do WM | `app.rs` |
| — | `--cols`/`--rows`/`--dpi`/`--no-vsync`/`--prof` | `cli.rs` |
