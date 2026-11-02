# Backlog — o que a migração não pede

Dois assuntos, duas listas, cada um com **um** dono:

- [`MIGRATION.md`](MIGRATION.md) — **paridade** com o Python. Escopo: fazer o
  Rust fazer o que o Python faz, e o que falta para isso. É a fila de trabalho
  da migração, com critério de pronto por fatia.
- **este arquivo** — o que fica **fora** do escopo da migração: melhoria além
  da paridade, e dívida de projeto que a paridade não resolve.

Os dois não se misturam: item de melhoria nasce aqui, com id `M-*`, e não
aparece no painel do `MIGRATION.md` (o painel conta paridade, não ambição). O
que já foi feito aparece nas tabelas de lá marcado `(**melhoria**)`, porque
virou capacidade do programa — mas a lista mora aqui.

## Regras da fila (as mesmas do bzmux Python)

- **Registrar é só registrar.** A investigação (ler código, medir, reproduzir)
  acontece quando o item for executado, não quando é anotado. Na anotação vai o
  sintoma, o gatilho e o que vier de graça no caminho; nada de diagnóstico
  completo antes da hora.
- **Cada item tem um gatilho.** Melhoria sem gatilho vira desejo: só entra na
  tabela o que tem uma frase do tipo "quando X aparecer". Item sem gatilho é
  limitação (`LIMITATIONS.md`), não fila.
- **Ao fechar um item**: sai da tabela daqui, ganha uma linha datada em
  [`HISTORY.md`](HISTORY.md) com o que foi verificado, e o que virou capacidade
  de paridade entra no painel do `MIGRATION.md` se lá houver linha para ele.
- **Bug corrigido fora de item** entra na lista numerada do `HISTORY.md`; se for
  armadilha de ambiente/ferramenta, entra em [`DEBUG.md`](DEBUG.md).
- **Decisão de não fazer** vai para [`LIMITATIONS.md`](LIMITATIONS.md).

## Aberto

| id | item | por que | gatilho | custo |
|---|---|---|---|---|
| M-Q | **Overlay de cromo** — barra de abas, cartões da sidebar e toast viram uma lista de elementos (retângulo + texto + área de clique) desenhada sobre a grade, no mesmo `wgpu` | hoje cada elemento de cromo é escrito **duas vezes**: o desenho em `render.rs` (`tabbar`/`sidebar`/`toast`) e o teste de clique em `app.rs` (`tab_at`/`card_at`), com as mesmas coordenadas em fórmulas separadas. Três bugs de uma sessão foram exatamente essa divergência: aba errada porque `tab_at` descontava a sidebar, cartão começando na linha errada, duplo clique no cartão da linha 1 virando clique de aba. Um `Vec<Elemento>` com hit-test declarado mata a duplicação sem toolkit e sem dependência de sistema (a grade continua célula a célula; só o cromo vira cena) | quando aparecer o **4º** elemento de cromo — menu de contexto, paleta de comandos, busca, alça de split, diálogo —, implementar **antes** de escrever o par desenho/clique pela quarta vez | baixo-médio: ~200 linhas (elemento, layout em célula, hit-test em ordem inversa de z) + migrar os 3 elementos existentes e apagar `tab_at`/`card_at`/`bar_drag` na forma atual |
| M-A | Render pelo dano por linha (A17/G17) | hoje monta instância de todas as linhas; a 61×18 custa 0,28 ms, mas em 4K/250×70 o custo passa a ser proporcional à tela, não à atividade | quando a medição em tela grande passar de ~1 ms de `build` | baixo: o core já marca `dirty[]`; falta filtrar no `build` e manter buffer persistente |
| M-C | OSC 8 hyperlink clicável com Ctrl+clique (C14/F17) | links em `dmesg`, compiladores e `ls --hyperlink`; o Python consome e joga fora | quando alguém reclamar que o link não abre | baixo: um `u32` por célula + hover |
| M-D | Busca no scrollback com realce (D11) | hoje só se acha rolando; em log grande é a função que falta | primeiro log de build que precise ser garimpado | médio: índice por linha ou varredura sob demanda |
| M-E | Seleção por palavra/linha e retangular (F10/F11) | copiar um caminho de log hoje exige arrasto preciso | quando copiar caminho por arrasto incomodar duas vezes | baixo |
| M-F | IME e composição (E11) | o Python não tem `IMContext`; quem digita CJK ou usa compose não consegue | primeiro usuário que digitar CJK | médio: `winit::Ime` + pré-edição na view |
| M-G | Kitty keyboard completo, incluindo flag 4 (E12-E19) | paridade com o que o bzmux promete (kitty/ghostty) e correção do `:2` (repeat) que o Gdk não entrega | quando um app perguntar por `CSI > u` e receber resposta incompleta | médio |
| M-H | Config v2: paleta como tema completo e remapeamento de atalhos (K20/K21) | o Python só tem 10 opções e cores fixas no código; o parser já está pronto | quando a paleta do arquivo não bastar (tema de 16 cores + cubo) | baixo-médio: falta o merge de paleta |
| M-I | Abas/workspaces persistentes (I18) | o Python perde o layout ao fechar; com `state.json` já existindo, é JSON a mais | quando fechar a janela custar o layout de um dia | baixo |
| M-J | Splits (I19) | pedido registrado no P39 do Python; aqui cada split é uma view no mesmo `wgpu` | quando dois terminais lado a lado forem rotina | alto |
| M-K | Notificação OSC 9/777 + urgency (C18) | "terminou o build" sem precisar olhar a janela | quando build longo exigir vigia | baixo |
| M-L | Imagens: z-index, placement id, placeholder, recorte de origem (H14-H16) | o Python mostra imagem sempre por cima e sem controle fino | depois de F6, com `icat` funcionando | médio |
| M-M | Barra de rolagem visível e arrastável (D9) | hoje só roda e `Shift+Pg` (que também falta) | quando rolar log grande só pela roda cansar | baixo: retângulo + estado |
| M-N | Transporte de socket no Windows (L14) | o `benchrun.py` e a automação precisam funcionar lá também | junto com F8 (portas) | baixo |
| M-P | Remapear a âncora da seleção no reflow (A21/A22) | hoje o `reflow` mata a seleção em vez de deslocá-la (o Python também mata). O `reflow` já calcula `(parágrafo, offset)` de cada linha velha, então a âncora nova é uma conta, não uma busca | quando perder uma seleção longa num resize incomodar | baixo: `reflow` devolve o mapa de linhas e `Span` é reescrito por ele |

## Fechados

- **M-O** `--selftest` — 14 checagens no binário, sem build de teste
  (`HISTORY.md`, 2026-11-02).
- **M-B** SGR completo — itálico, faint, blink, riscado, overline, cor e estilos
  de sublinhado (`HISTORY.md`, 2026-11-02).
