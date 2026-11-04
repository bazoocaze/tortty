# Migração: bzmux Python → bzmux Rust

Painel de controle da migração. Uma linha por funcionalidade do bzmux Python,
com o estado dela aqui. É por este arquivo que se responde "quanto falta" — e
não por estimativa.

- **O Python foi a especificação observável** (`~/tortty-python`):
  cada linha guarda o `arquivo:linha` de onde a funcionalidade veio.
- **A migração está fechada** (2026-11-02): **200 linhas, todas `feito`**. Enquanto
  ela corria, o que existia lá e não existia aqui era `falta`, não limite; hoje
  o que falta aqui é **atividade nova** e mora no [`BACKLOG.md`](BACKLOG.md), com
  id `B<numero>` (os antigos continuam `M-*`). O que é decisão de **não** fazer
  mora em [`LIMITATIONS.md`](LIMITATIONS.md) e aqui apareceria como `não`.
- Número de desempenho não entra aqui: [`BENCHMARKS.md`](BENCHMARKS.md).
- Registro datado do que já foi fechado: [`HISTORY.md`](HISTORY.md).

## Painel
| área | feito | parcial | falta | não | impedida | total |
|---|---|---|---|---|---|---|
| 1. Núcleo: célula, cores, grade | 18 | 0 | 0 | 0 | 0 | 18 |
| 2. Parser: controles, CSI, modos | 28 | 0 | 0 | 0 | 0 | 28 |
| 3. Parser: SGR e OSC | 13 | 0 | 0 | 0 | 0 | 13 |
| 4. Scrollback e visão do histórico | 8 | 0 | 0 | 0 | 0 | 8 |
| 5. Entrada: teclado e protocolo do kitty | 16 | 0 | 0 | 0 | 0 | 16 |
| 6. Entrada: mouse, seleção, clipboard, paste | 14 | 0 | 0 | 0 | 0 | 14 |
| 7. Desenho: fonte, geometria, cores | 15 | 0 | 0 | 0 | 0 | 15 |
| 8. Imagens do protocolo kitty | 13 | 0 | 0 | 0 | 0 | 13 |
| 9. Janela: abas, workspaces, cromo | 18 | 0 | 0 | 0 | 0 | 18 |
| 10. Janela: ciclo de vida, geometria, estado | 10 | 0 | 0 | 0 | 0 | 10 |
| 11. Configuração e CLI | 18 | 0 | 0 | 0 | 0 | 18 |
| 12. Debug socket e instrumentação | 14 | 0 | 0 | 0 | 0 | 14 |
| 13. Plataforma e distribuição | 6 | 0 | 0 | 0 | 0 | 6 |
| 14. Testes e verificação | 9 | 0 | 0 | 0 | 0 | 9 |
| **total** | **200** | **0** | **0** | **0** | **0** | **200** |

Equivalente pronto: **100%** (200 de 200 linhas de paridade `feito`; `parcial`
conta 0,5 e `impedida` conta como `falta` — nenhuma das duas existe mais). O total
aqui é só paridade: melhoria que o Python não tem mora no
[`BACKLOG.md`](BACKLOG.md) e não entra nesta conta.

O que veio depois da migração — e a ordem em que ela foi feita — está na seção 16.

## Regras deste documento

1. **Uma linha, uma funcionalidade.** Se a linha precisa de "e" para descrever,
   são duas linhas.
2. **Estado sempre preenchido** com um destes: `feito`, `parcial`, `falta`,
   `não`, `impedida`. `parcial` exige dizer **o que falta** na própria linha;
   `impedida` exige dizer **o que impede** (ambiente que não existe aqui), e não
   é o mesmo que `não`: `não` é decisão, `impedida` é falta de condição. **Melhoria
   não tem estado aqui**: se o Python não faz, não é paridade — vira item do
   [`BACKLOG.md`](BACKLOG.md) e não ganha linha neste arquivo.
3. **Fechar item** = mudar o estado, rodar a verificação da coluna
   "verificação" e registrar uma linha em `HISTORY.md` com a data. O item não
   sai daqui: o painel conta o que está feito, não o que sobrou.
4. **Item novo** (descoberto no meio do caminho) entra como linha nova na área,
   não como parágrafo. Se o Python não tem, é **melhoria** (`BACKLOG.md`), não
   migração.
5. **Ordem de execução** não mora nas tabelas: mora na seção 16. Marcar `feito`
   fora de ordem é normal; mudar a ordem é decisão registrada.
6. **Verificação** é o que dá direito a `feito`: teste automatizado quando
   existe equivalente no `checks.py` do Python; senão, o roteiro manual escrito
   na própria linha (`vim`, `mc`, `dmesg`, captura de tela).

Legenda das colunas `ref`: `arquivo:linha` do repo Python → `arquivo` do repo
Rust (sem linha, para não envelhecer a cada commit).

---

## 1. Núcleo: célula, cores, grade

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| A1 | Célula `{ch, fg, bg, flags}` | `palette.py:1-6` → `cell.rs` | feito |
| A2 | Paleta 16 ANSI + 8 brights | `palette.py:38-55` → `color.rs` | feito |
| A3 | Cubo 6×6×6 + 24 cinzas (xterm-256) | `palette.py:58-70` → `color.rs` | feito |
| A4 | Default fg `#d6d6d6` / bg `#000000` | `palette.py:71-76` → `color.rs` | feito |
| A5 | Truecolor na célula (sem tabela alocada) | `palette.py:24-32` → `color.rs` | feito |
| A6 | Sentinela de cor default distinta de índice | `palette.py:15-38` → `color.rs` | feito |
| A7 | Reverse video por célula (inversão depois de resolver cor) | `view.py:963-973` → `render.rs` | feito |
| A8 | BCE: apagar herda o bg corrente | `term.py:155-157` → `term.rs` | feito |
| A9 | Charset DEC Special Graphics (G0, `ESC ( 0`) | `palette.py:110-142` → `charset.rs` | feito |
| A16 | Dano por linha (`dirty`/`dirty_all`) | `term.py:355-362` → `term.rs` | feito |
| A17 | Render usa o dano (só remonta as linhas sujas) | `view.py:920-935` → `damage.rs` + `render.rs` | feito: `damage.rs` decide por `Signature` (aba, geometria, rolagem, seleção, pisca, tema) e o renderer guarda instância por linha. Rolagem de tela cheia passou a **deslocar** as linhas prontas (o núcleo reporta `scroll_shift`), e `IL`/`DL`/`ED0`/`ED1` marcam só a faixa afetada. Medido em grade 60x12: `seq 1 100` remonta 12 linhas uma vez, `echo` remonta 1 (`state` → `rows_built`, `frames_all`, `global_why`); antes era a grade inteira a cada linha escrita. Conferido na tela (`seq`, `mc`) |
| A18 | Grade de alt-screen separada, com troca preservando cursor | `term.py:602-629` → `term.rs` | feito |
| A19 | Alt-screen no resize: recorte/preenchimento sem reflow | `term.py:184-217` → `term.rs` | feito |
| A20 | Linha que sai pelo topo vai para o scrollback (só `top==0`) | `term.py:384-400` → `term.rs` | feito |
| A21 | Âncora absoluta (`scrolled` + geração de mapa/limpeza) | `term.py:99-108,1026-1078` → `select.rs` (`gen`) | feito: reflow/ED3/RIS sobem `abs_gen` e matam a âncora |
| A22 | Reflow no resize (buffer principal, junta/parte linha mole) | `term.py:219-332` → `term.rs` `reflow()` | feito |
| A23 | Reflow preserva bg colorido (corta só branco com bg default) | `term.py:270-280` → `Cell::is_blank` no corte | feito |
| A24 | Núcleo sem I/O, sem GUI, sem relógio (testável headless) | decisão do Python (`AGENTS.md`) → `bzmux-core` | feito |

## 2. Parser: controles, CSI, modos

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| B1 | C0: BEL (contador), BS, HT, LF, CR | `term.py:408-441` → `term.rs` | feito |
| B2 | C0: CAN/SUB cancelam sequência | `term.py:858-861` → `term.rs` | feito |
| B3 | C1 (0x80-0x9F) | desenha como texto (`term.py:862-866`) | feito: C1 (0x80-0x9F) desenha como texto, igual ao Python — conferido `0xC2 0x80`/`0xC2 0x9B` no meio de "abc": 5 células, `a\u{80}b\u{9b}c`. `CSI` de 8 bits não é interpretado em nenhum dos dois |
| B4 | `ESC [ ` CSI, `ESC ]` OSC, `ESC P/_/^/X` DCS/APC/PM/SOS | `term.py:871-876` → `term.rs` | feito |
| B6 | `ESC 7`/`ESC 8` DECSC/DECRC | `term.py:882-888` → `term.rs` | feito |
| B9 | `ESC M` RI (scroll reverso na margem) | só move cursor (`term.py:879-881`) → `term.rs` | feito |
| B10 | `ESC H` HTS, `ESC c` RIS | RIS `term.py:889-890` → `term.rs` | feito |
| B12 | CSI cursor: CUU/CUD/CUF/CUB/CNL/CPL/CHA/VPA/CUP/HVP | `term.py:688-703` → `term.rs` | feito |
| B13 | CSI `s`/`u` (SCOSC/SCORC) e `u` com prefixo = kitty kbd | `term.py:734-740` → `term.rs` | feito: `CSI s`/`u` clássicos intactos e `u` com prefixo é o protocolo do kitty (`?`/`>`/`<`/`=`) |
| B14 | IL/DL/ICH/DCH/ECH | `term.py:708-728` → `term.rs` | feito |
| B16 | EL 0/1/2 e ED 0/1/2 | `term.py:704-707` → `term.rs` | feito |
| B17 | ED 3 limpa scrollback + invalida âncoras | `term.py:456-465` → `term.rs` | feito |
| B19 | DECSTBM (região de scroll) | `term.py:729-733` → `term.rs` | feito |
| B21 | Modos privados `?1` DECCKM, `?25` DECTCEM | `term.py:594-601` → `term.rs` | feito |
| B22 | `?47`/`?1047`/`?1049` alt screen (+ limpeza de histórico no 1049) | `term.py:602-629` → `term.rs` | feito |
| B23 | `?1048` save/restore cursor | `term.py:630-634` → `term.rs` | feito |
| B24 | `?1000`/`?1002`/`?1003` (estado do mouse) | `term.py:635-639` → `term.rs` | feito (estado; a codificação falta — E) |
| B25 | `?1006` SGR mouse (estado) | `term.py:640-641` → `term.rs` | feito (estado; codificação falta) |
| B26 | `?1004` focus report (estado + envio no app) | `term.py:642-643` → `term.rs`, `app.rs` | feito |
| B27 | `?2004` bracketed paste + payload `200~`/`201~` | `term.py:648-651,1082-1091` → `select.rs` | feito |
| B28 | `?2026` synchronized output | `term.py:652-655` | feito: estado no parser e o frame é segurado até o fecho (`frame::Gate`), com teto de 200 ms. Verificado no pty: 20 linhas dentro da transação → 4 desenhos em 0,4 s (uma por teto), e o estado final sai no fecho |
| B29 | DSR/CPR `CSI 6 n`, status `CSI 5 n` | `term.py:772-773` → `term.rs` | feito |
| B30 | DA1 `CSI c`, DA2 `CSI > c` | `term.py:743-749` → `term.rs` | feito |
| B31 | DECRQM `CSI ? … $ p` | `term.py:750-754` → `term.rs` | feito |
| B32 | `CSI 16 t` (célula) e `CSI 18 t` (grade) | `term.py:755-762` → `term.rs` | feito |
| B33 | DECSCUSR `CSI Ps SP q` (+ volta à config no 0) | `term.py:764-771` → `term.rs`, `render.rs` | feito |
| B34 | Buffer de resposta ao filho (`child_out`) | `term.py:140` → `term.rs` | feito |
| B35 | UTF-8 incremental com parcial bufferizado e inválido → U+FFFD | `view.py:227,404-406` → `term.rs` | feito |

## 3. Parser: SGR e OSC

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| C1 | SGR 0, 1, 4, 7, 22, 24, 27 | `term.py:514-527` → `term.rs` | feito |
| C2 | SGR 30-37, 39, 40-47, 49, 90-97, 100-107 | `term.py:528-539` → `term.rs` | feito |
| C3 | SGR 38;5 / 48;5 (256) e 38;2 / 48;2 (truecolor) | `term.py:540-576` → `term.rs` | feito |
| C4 | Subparâmetros `:` (`38:2::r:g:b`) | `term.py:500-512,579-591` → `term.rs` | feito |
| C5 | SGR 3 itálico, 2 faint, 5/6 blink, 9/29 riscado | não existem (`LIMITATIONS.md`) → `cell.rs`, `render.rs` | feito (SGR 5 anima: fase de 500 ms em `Blink`) |
| C6 | SGR 53/55 overline, 58/59 cor do sublinhado | não existem → `term.rs` | feito |
| C7 | SGR 4:2/4:3/4:4/4:5 (estilos de sublinhado) | `4:x` colapsa em simples (`term.py:518-519`) → `render.rs` | feito (curvo é aproximação em 4 retângulos) |
| C8 | Bold sem "bold as bright" | `view.py:992` → `render.rs` | feito (mesma semântica) |
| C9 | OSC 0/2 título | `term.py:959-960` → `term.rs` | feito |
| C10 | OSC 1 (título de ícone) | ignorado no Python | feito (ignorado, igual) |
| C11 | OSC 10/11 query de cor default | `term.py:961-965` → `Term::osc_color` | feito |
| C12 | OSC 4 query de paleta | `term.py:966-980` → `Term::osc_palette` | feito |
| C19 | OSC com terminador BEL **e** ST, ESC solto volta a reprocessar | `term.py:914-926` → `term.rs` | feito |

## 4. Scrollback e visão do histórico

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| D1 | Scrollback com teto configurável (default 1000) | `term.py:109`, `config.py:150` → `term.rs` | feito |
| D2 | Flag de quebra mole paralela ao histórico | `term.py:106-118` → `grid.rs` | feito |
| D3 | `visible_rows(off)` sempre devolve `rows` linhas | `term.py:1006-1024` → `term.rs` | feito |
| D4 | Roda do mouse rola 3 linhas | `view.py:1208-1226` → `app.rs` | feito |
| D6 | Shift+PgUp/PgDn/Home/End | `view.py:1285-1292` → `app.rs` | feito |
| D7 | Qualquer tecla volta ao vivo (modificador não conta) | `view.py:1290-1292` → `app.rs` | feito |
| D8 | Indicador `↑N` no canto quando no histórico | `view.py:1082-1091` → `render.rs` | feito |
| D10 | Visão fixada ao ler enquanto chega output | `view.py` + `term.py:1026-1078` → `Term::view_after_output` | feito |

## 5. Entrada: teclado e protocolo do kitty

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| E1 | Tabela de teclas simples (setas, Home/End, Ins/Del, PgUp/PgDn, F1-F12) | `keys.py:46-74` → `keys.rs` | feito |
| E2 | Modificador como parâmetro (`CSI 1;mA`) | `keys.py:116-138` → `keys.rs` | feito |
| E3 | DECCKM: setas/Home/End em modo aplicação (`ESC O A`) | `keys.py:78-84` → `keys.rs` | feito |
| E4 | Shift+Tab (`ISO_Left_Tab` → `CSI Z`) | `keys.py:180-181` → `keys.rs` | feito |
| E5 | Ctrl+letra (`chr(kv&0x1f)`), Ctrl+Space, Ctrl+minus | `keys.py:192-199` → `keys.rs` | feito (Ctrl+Space = NUL, Ctrl+\ = 0x1c) |
| E6 | Alt como prefixo ESC em tecla especial e em texto | `keys.py:121-138,215-216` → `keys.rs` | feito (tecla especial vira `CSI 1;3x`, texto vira `ESC` + letra) |
| E9 | Dead keys / composição (ABNT2 `~ ^ ´`) | `keys.py:13-44,219-227` → winit | feito (o winit entrega o texto composto; `Key::Dead` não gera byte) |
| E10 | Texto imprimível vindo do evento (layout não-US) | `view.py:1295-1300` → `app.rs` | feito (usa `event.text` e cai em `acquired` quando vazio) |
| E12 | Kitty kbd: `CSI > u` push, `CSI < n u` pop, `CSI = f;m u` set | `term.py:982-1001` | feito: `CSI > u` empilha (teto de 64, como o kitty), `CSI < n u` desempilha; `Term::kbd_flags` guarda por aba |
| E13 | Kitty kbd: `CSI ? u` query | `term.py:986` | feito: `CSI ? u` responde `x1b[?<flags>u`; conferido ao vivo (`?0u`, `?3u`, `?11u` após cada push) |
| E14 | Kitty kbd flag 1 (disambiguate) | `kitty_kbd.py:25-52,115-119` | feito: flag 1 — ESC vira `CSI 27u`, Ctrl+a `CSI 97;5u`, seta com modificador `CSI 1;5A`; seta sem modificador continua no legado (respeita DECCKM) |
| E15 | Kitty kbd flag 2 (event types, release `:3`) | `kitty_kbd.py:90-109` | feito: flag 2 — `:1`/`:2`/`:3`; tecla segurada deu `CSI 97;1:2u` ×5 e release `:3` no pty (hex lido da tela) |
| E17 | Kitty kbd flag 8 (all keys as CSI u) | `kitty_kbd.py:127-135` | feito: flag 8 — `a` vira `CSI 97u`, Shift+letra deixa de ser texto puro |
| E18 | Kitty kbd flag 16 (associated text) | `kitty_kbd.py:62-79` | feito: flag 16 — `CSI 97;97u` (campo de texto depois do código) |
| E19 | Release sem forma kitty não manda texto (bug do `pi`) | `kitty_kbd.py:90-99` | feito: release sem a flag 2 não manda nada; a soltura só chega ao encoder quando a flag existe (era o texto dobrado no `pi`) |
| E20 | Atalhos locais têm precedência sobre o encoder | `view.py:1263-1267` | feito: atalho local roda antes do encoder **e** come a soltura da tecla (senão `Ctrl+Shift+1` deixava `CSI 49;1:3u` no filho; o X11 solta Shift/Ctrl antes da tecla) |

## 6. Entrada: mouse, seleção, clipboard, paste

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| F1 | Encoding X10 legado (`CSI M` + 3 bytes, release=3, coord ≤222) | `mouse.py:57-64` → `mouse.rs` | feito |
| F2 | Encoding SGR (`CSI < b;x;y M/m`) | `mouse.py:51-64` → `mouse.rs` | feito |
| F3 | Modificadores no código do botão (+4/+8/+16) | `mouse.py:26,34-36` → `mouse.rs` | feito |
| F4 | Roda como botões 64-67 (press e release no SGR) | `mouse.py:21-23,46-47` → `mouse.rs` | feito |
| F5 | Botões 8/9 (back/forward) | `mouse.py:22,30` → `mouse.rs` | feito |
| F6 | Motion com botão (`?1002`) e sem botão (`?1003`) | `view.py:1199-1205` → `app.rs` | feito |
| F8 | Seleção por arrasto com limiar de 3 px | `view.py:1145-1200` → `app.rs` | feito |
| F9 | Seleção em âncora absoluta (sobrevive a rolagem e output) | `view.py:854-881`, `term.py:1056-1075` → `select.rs` | feito (o reflow da A21 ainda mata a âncora, igual ao Python) |
| F12 | Seleção: cópia no release para PRIMARY; `copy_on_select` copia CLIPBOARD | `view.py:1168-1176` → `app.rs` | feito: no release copia PRIMARY sempre e CLIPBOARD só com `copy_on_select`; sem arrasto real (`a == b`) não copia e o PRIMARY do usuário fica intacto — mesma condição do `view.py:1168-1176` |
| F13 | Ctrl+Shift+C copia (seleção ou viewport), Ctrl+Shift+V cola | `view.py:1271-1277,1239-1254` → `app.rs` | feito |
| F14 | Botão 2 cola PRIMARY | `view.py:1152-1154` → `clip.rs` | feito |
| F15 | Bracketed paste envolve payload em `200~`/`201~` | `term.py:1082-1091` → `select.rs` | feito |
| F16 | Clipboard sobrevive ao exit / integração com o sistema | `Gtk.Clipboard` → `clip.rs` (`arboard`) | feito no CLIPBOARD; PRIMARY morre com o app (é o dono da seleção) |
| F19 | Clique com mouse mode ativo volta ao vivo antes de reportar | `view.py:1136-1141` → `app.rs` | feito |

## 7. Desenho: fonte, geometria, cores

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| G1 | Fonte em **pontos** convertida por DPI (equivalente ao Pango) | `view.py:640-659` → `glyph.rs` | feito |
| G2 | Métrica da célula pelo avanço de `M` (não pela caixa tipográfica) | `view.py:661-680` → `glyph.rs` | feito |
| G3 | Baseline do layout, não ascent fixa | `view.py:1011-1021` → `glyph.rs` | feito |
| G4 | Célula inteira (avanço fracionário não "escorrega" a grade) | `view.py:689-704` → `render.rs` | feito |
| G5 | Negrito e itálico (face real; negrito-itálico próprio) | `view.py:689-704,992` → `glyph.rs` | feito (sem síntese: sem face, cai na regular) |
| G6 | Fallback de glifos por lista de preferência | Pango/fontconfig automático → `glyph.rs` | feito |
| G7 | Fallback para emoji/CJK | `HISTORY.md` P10 → `glyph.rs`, `atlas.rs`, `render.rs` | feito: cadeia de socorro por nome (fonte ausente não vira outra) e duas passadas por glifo (cor antes de máscara); emoji CBDT vai para atlas RGBA próprio, desenhado pela pipeline de imagem. Na tela: 😀🔥🚀👍 e CJK, setas, grego; `emoji_sai_colorido` e `fonte_ausente_nao_vira_outra` no `cargo test` |
| G8 | Ligaduras/shaping | Pango (P9) → `shape.rs` | feito: `rustybuzz` por trecho de mesma face/estilo, com mapa cluster → célula e cache por (face, modo, texto). Ligadura em fonte mono não funde célula (o `calt` troca o glifo de cada uma), então a célula continua sendo um caractere. Chave `ligatures` (`off`/`standard`/`all`); cursor e seleção quebram a ligadura. Na tela: JetBrains Mono com `->`, `!=`, `fi`, `ffi`, `fl` (Xvfb) |
| G9 | Padding da janela em px | `config.py` `window_padding_width` → shader (`origin`) | feito |
| G10 | Fundo da sobra de célula até a borda (sem tarja preta) | `view.py:926-934` | feito (fundo pintado por instância de grade inteira) |
| G12 | Cursor: block/beam/underline + pisca | `view.py:1058-1079`, `828-838` → `render.rs`, `app.rs` | feito: `cursor_blink on` pisca a 500 ms, tecla/colagem reacende, `CSI 6 q` par vence; SGR 5 usa a mesma fase |
| G13 | Sublinhado na largura do run | `view.py:1022-1025` → `render.rs` | feito |
| G14 | Seleção desenhada como inversão | `view.py:943,953-973` → `render.rs` | feito (o default resolve pelo outro extremo) |
| G15 | `?2026` segura o frame (teto 200 ms) | `view.py:296-324` | feito: `frame::Gate` (`SYNC_MS = 200`), testado sem display (transação aberta, fecho imediato, app que nunca fecha não desenha a cada leitura) |
| G16 | Frame coalescido (~16 ms), não um desenho por leitura | `view.py:286-320` | feito: `frame::Gate` (`FRAME_MS = 16`), alimentado no `Wake` **depois** do `drain` (o `?2026` só é visível após parsear). 900 frames: 1499–1585 desenhos → **63–74**, mesmo fps (`BENCHMARKS.md`) |

## 8. Imagens do protocolo kitty

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| H1 | APC `ESC _ G … ST` roteado para o subsistema | `term.py:944-953` | feito: `State::Str` ganhou o buffer de APC (`é só o APC`: DCS/PM/SOS seguem descartados) e o ST entrega o corpo ao `graphics::handle` |
| H2 | `a=t` transmitir / `a=T` transmitir+exibir | `graphics.py:188-212` | feito: `a=t` guarda a imagem, `a=T` guarda e coloca. Verificado com `kitten icat` de verdade (`blueman.png` e dois `icat` na mesma tela) |
| H3 | `a=p` exibir imagem já transmitida | `graphics.py:184-192` | feito: coloca a imagem por id; id inexistente responde `ENOENT` (como o Python) |
| H4 | `a=d` apagar (por id e tudo) | `graphics.py:175-183` | feito: `d=i` remove a imagem e as colocações dela, `d=a` limpa tudo (e o app solta a textura no frame seguinte) |
| H5 | `a=q` query de capacidade | `graphics.py:173-181` | feito: responde `OK` para `t=d` com `f=24/32/100` e `EINVAL` para transmissão por arquivo/memória — é essa resposta que faz o `icat` seguir em vez de desistir |
| H6 | Chunking `m=1` | `graphics.py:146-155` | feito: o control data do **primeiro** pedaço vale para a transmissão e o `m` não acumula (era o bug do Python documentado lá); teto de 64 MiB para pedaço que nunca fecha |
| H7 | `f=24/32` (RGB/RGBA cru) e `f=100` (PNG) | `graphics.py:190-196` | feito: os três no core; o PNG é decodificado no app (`png` crate) e o magic do arquivo vence o `f=` mentiroso |
| H8 | `o=z` (zlib) | `graphics.py:161-168` | feito: `flate2` (zlib, não gzip); zlib torto responde `EINVAL:zlib` em vez de derrubar o parser |
| H9 | `s=`/`v=`, `i=` automático, `c=`/`r=` default, `X=`/`Y=` offset px | `graphics.py:100-106,193-207` | feito: `c`/`r` default = `ceil(largura/célula)`; sem `i=` o id sai de um contador. **Corrigido no caminho:** o app nunca passava o tamanho real da célula ao parser (ficava no padrão 10×19), então a imagem saía 5×3 onde cabiam 4×2 |
| H10 | `C=1` não move o cursor; senão desce `rows` | `graphics.py:184-187` | feito: `Saida.mover`/`linhas` e o `apc_done` desce o cursor às linhas da imagem; `C=1` deixa onde está |
| H11 | `q=` silencia resposta | `graphics.py:170` | feito: qualquer `q` diferente de 0 cala a resposta (o `icat` manda `q=2`) |
| H12 | Imagem acompanha a rolagem e é podada do histórico | `graphics.py:74-96` | feito: a colocação vive em linha de tela (negativa = histórico); `scroll_up` desce todo mundo, rolagem de região descarta, teto do scrollback e reflow podam o inalcançável. No desenho a linha na tela é `linha + view_off` — verificado com screenshot: a imagem volta no lugar certo ao subir no histórico |
| H13 | `ED 3` limpa imagens | `term.py:481-485` | feito: `CSI 3 J` limpa imagens e colocações junto com o histórico; `ED 2` **não** limpa (verificado nos dois sentidos) |

## 9. Janela: abas, workspaces, cromo

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| I1 | Aba = termo + pty com ciclo de vida próprio | `window.py:140-211` → `app.rs:Tab` | feito: `Tab` tem term/pty/rx/visão/seleção; `drain` lê todas |
| I2 | Nova aba herda cwd da aba corrente (último descendente do pty) | `window.py:140-153,904-919` | feito: `/proc/<pid>/cwd` do descendente mais fundo; `cd /tmp` → aba nova em `/tmp` |
| I3 | Fechar aba (fecha pty, remove página, escolhe a próxima) | `window.py:184-201` | feito: `Ctrl+Shift+W`; aba de shell morto fecha sozinha |
| I4 | Título da aba por OSC 0/2, senão `basename(cwd)`, senão `bzmux` | `window.py:277-286` | feito: `Tab::title` (OSC → cwd → `bzmux`) |
| I5 | Workspace = conjunto de abas, com nome e renumeração | `window.py:798-848` → `app.rs:Workspace` | feito: `parked` guarda os outros inteiros (pty vivo); trocar é swap; ordem lógica estável |
| I6 | Sidebar com cartões (`n`, nome, título, k abas) | `window.py:289-301` → `render.rs:sidebar` | feito: coluna de 18 células; cartão ativo invertido; título trunca com `…` |
| I7 | Sidebar escondida com 1 workspace | `window.py:396-400` | feito: `side_w` = 0 com um workspace; a grade ganha as colunas de volta |
| I8 | Renomear workspace inline (duplo clique, Enter/Esc) | `window.py:315-356` | feito: duplo clique no cartão; 1ª tecla substitui (como o `Gtk.Entry` selecionado), Enter grava, Esc descarta, `\|` é o cursor |
| I9 | Clique do meio fecha aba (no rótulo) e workspace (no cartão) | `window.py:208-212,307-309` | feito no Rust (o Python **não** tinha: só o cartão); o clique esquerdo troca |
| I10 | Arrastar aba para reordenar | `window.py:173` → `app.rs:bar_drag` | feito: anda uma aba de largura com o botão apertado na barra e a aba troca de lugar |
| I11 | Roda do mouse na barra de abas | `window.py:100` | feito: circula entre as abas; o X11 manda press+release por entalhe, e o par é deduplicado (10 ms) |
| I12 | Toast (erro/aviso) com fila e timeout | `window.py:411-465,677-740` → `render.rs:toast` | feito: faixa invertida na **última** linha (o topo é da barra), 3,5 s, repetida vira `(×N)` |
| I13 | Bell visual: flash do rótulo por 800 ms (800 ms, `BELL_BG`) | `window.py:255-275` → `render.rs:tabbar` | feito: `color3` por 800 ms, inclusive em aba de segundo plano |
| I14 | Bell audível (`beep`) | `window.py:666-669` | feito (`bell audible` escreve BEL no stdout; sem `beep` de toolkit) |
| I15 | Atalhos de aba/workspace (Ctrl+Shift+T/W/N/Q/F2, Alt+N, Super+N/Q) | `window.py:854-898` | feito: Ctrl+Shift+T/W/N/Q/F2, Ctrl+Shift+1..9, Alt+1..9, Super+N/Q/1..9 |
| I16 | Dígito de atalho por keycode cru (sobrevive a ABNT2) | `window.py:70-84` → `digit_of` (tecla física) | feito |
| I17 | Título da janela = título da aba ativa | `window.py:375,921-926` | feito: `<título> — bzmux`, só quando muda |
| I20 | Fechar última aba fecha o workspace; último workspace fecha o app | `window.py:196-197,836-845` | feito: o atalho e a **saída do filho** entram no mesmo caminho (`close_tab_at` → `drop_empty_workspace`). Verificado ao vivo: 2 workspaces × 1 aba, `exit` na visível → promoção do estacionado e app vivo; 1 workspace × 1 aba → app fecha |

## 10. Janela: ciclo de vida, geometria, estado

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| J1 | Fecha a janela quando o shell termina | `window.py:250-253` → `app.rs` | feito |
| J2 | Geometria inicial 980×640 e restauração de tamanho | `window.py:24-31,467-478` | feito: sem estado abre 980×640 (conferido com `xdotool`: `Geometry: 980x640`); com estado restaura o tamanho (900×600 pedido → 900×600). Quem pede grade (`--cols/--rows`) **não** restaura nem salva — é o papel do `PYTERM_GEOM` lá, e é o que a automação e o bench precisam |
| J3 | Persistência de geometria (`$XDG_STATE_HOME/bzmux/window.json`) | `state.py:16-63` | feito: `state.rs` (puro), gravação atômica (tmp + rename, sem tmp sobrando — testado), `XDG_STATE_HOME` com `~/.local/state` de reserva e erro de I/O engolido no fechamento |
| J4 | Rejeitar geometria inválida (monitor desconectado, lixo) | `state.py:46-63` | feito: `state::fit` exige `w/h` ≥ 320×200 e o canto **dentro de algum monitor**. Ao vivo: 100×80 → 980×640, canto em 9000,9000 → 980×640, JSON torto → 980×640, e o arquivo é reescrito são |
| J5 | Maximizado é estado separado do tamanho normal | `window.py:1009-1012` | feito: `Resized` só atualiza o tamanho normal quando a janela **não** está maximizada; o arquivo guarda `max` ao lado de `w/h`. Ao vivo: maximizado 1920×985 → arquivo `{w:900,h:600,max:true}` → reabre maximizado → desmaximizar volta a 900×600 |
| J6 | Wayland: não salvar x/y | `window.py:975-978` | feito: `backend_x11()` decide por `WAYLAND_DISPLAY`/`DISPLAY`; sem X11 a posição não é pedida nem gravada, e `state::save` simplesmente omite `x`/`y` (testado). Ressalva: não há sessão Wayland nesta máquina, então esta linha é código + teste, não verificação ao vivo |
| J7 | `SIGUSR1` = dump imediato; `SIGTERM` = sair limpo | `window.py:503-508` | feito: `SIGUSR1` = dump na hora, `SIGTERM` = dump e saída limpa (handler só levanta `AtomicBool`; quem age é o laço) |
| J8 | `--quit-after SEG` (usado pela automação) | `cli.py:35` | feito: `--quit-after SEG` fecha sozinho e deixa o dump (conferido: dump escrito e processo fora) |
| J9 | Dump do texto de todas as abas para arquivo | `window.py:929-945` | feito: dump com cabeçalho por workspace e por aba, scrollback (últimas 40) e viewport, marcando a aba `[fundo]` |
| J11 | Uma instância por PID file, sem matar processo alheio | `debug.sh` (Python) → `tools/devkill.sh` | feito |

## 11. Configuração e CLI

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| K1 | Formato kitty (`chave valor`, `#` comenta, repetida vence a última) | `config.py:66-85,264-287` → `config.rs` | feito |
| K2 | Merge com config default embarcada | `config.py:289-305` → `DEFAULT_CONF` | feito |
| K3 | Erro por linha não derruba o arquivo; chave desconhecida é erro | `config.py:264-287` → `Config::Error` | feito |
| K4 | `shell` (vazio = `$SHELL`; exporta o shell real ao filho) | `config.py:87-97` → `pty.rs` | feito |
| K5 | `font_family`, `font_size` | `config.py:99-125` → `cli.rs` + renderer | feito (CLI vence o arquivo) |
| K6 | `cursor_shape`, `cursor_blink` | `config.py:127-143` → `render.rs` | feito (forma e pisca; reload a quente) |
| K7 | `scrollback` | `config.py:149-151` → `cli.rs` | feito |
| K8 | `window_padding_width` | `config.py:153-155` → shader + `cell_at` | feito |
| K9 | `bell` (`none`/`visual`/`audible`) | `config.py:131-133` → `App::ring` | feito |
| K10 | `copy_on_select` | `config.py:145-147` → `app.rs` | feito |
| K11 | `debug_log` (path ou `off`) | `config.py:157-167` → `cli.rs` | feito |
| K12 | Auto-reload com debounce de 250 ms | `config.py:307-329` → `App::check_config` | feito |
| K13 | Erros de config viram toast (1º + contagem) | `config.py:331-345` | feito: arranque e reload mostram "config mantida — linha N: 'chave': motivo (+K)"; salvar certo mostra "config recarregada" |
| K14 | `-c/--config`, `--cmd`, `--debug-socket`, `--debug-log` | `cli.py:31-41` → `cli.rs` | feito |
| K15 | `--quit-after`, `--dump-to`, `--selftest`, `--install-shortcut` | `cli.py:35-96` | feito: `--selftest`, `--quit-after`, `--dump-to` (o atalho de desktop é do `install.sh`) |
| K16 | `--version`, `--help`, código 2 para uso inválido | `cli.py:85-101` → `cli.rs` | feito |
| K18 | `.desktop` + ícone (`--install-shortcut`) | `shortcut.py:34-136` | feito: `.desktop` + SVG saem do `install.sh` (com `StartupWMClass`); não existe `--install-shortcut` como flag do binário — o instalador é o dono, ter os dois seria segunda fonte |
| K19 | WM_CLASS estável (`StartupWMClass`) | `cli.py:130-131` | feito: `WM_CLASS` explícito (`with_name("bzmux","bzmux")`), não herdado do executável — conferido rodando o binário renomeado (`/tmp/fz…`): `WM_CLASS(STRING) = "bzmux", "bzmux"` |

## 12. Debug socket e instrumentação

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| L1 | Transporte Unix socket, JSON por linha, vários pedidos por conexão | `debugsock.py:217-240` → `debugsock.rs` | feito |
| L2 | `state` com grade, cursor, título, histórico, célula, fonte, dpi | `window.py:531-546` → `app.rs` | feito: `state` traz grade, cursor, título, histórico, célula, fonte, `pid`, `xid` (Xlib/Xcb; 0 no Wayland), `config`, `errors`, `dpi`, `advance`, `padding`, `pin`, seleção e métricas |
| L3 | `text` (`view`/`history`/`screen`, `start`, `count`) | `debugsock.py:86-93` | feito: `view`/`history`/`screen` com `start`/`count` (`screen` é a grade viva, sem o offset da visão) |
| L4 | `write` (bytes crus no pty) | `debugsock.py:106-111` | feito |
| L5 | `keys` (spec → teclas, passando pelo encoder e atalhos) | `debugsock.py:94-99` → `app.rs` | feito: `keys` passa pelo **mesmo** caminho do teclado (atalho local antes do encoder, `handled` na resposta); `ctrl+shift+n` volta `handled:true` sem byte |
| L6 | `resize` | `debugsock.py:100-103` | feito |
| L7 | `prof` (contadores por etapa) | `debugsock.py:104-105` | feito: `draw`/`parse`/`pty` + `writes`/`write_kb`/`eagain` (contador na escrita do pty) |
| L8 | `log` (ligar/desligar log em runtime) | `debugsock.py:112-113` | feito: `log` liga, desliga (`off`) e troca o caminho em execução, valendo também como config corrente |
| L9 | `quit` (resposta sai antes de fechar) | `debugsock.py:114-115` | feito |
| L10 | `--debug-socket auto` (`$XDG_RUNTIME_DIR/bzmux-<pid>.sock`) | `debugsock.py:33-43` → `debugsock.rs` | feito |
| L11 | Socket `0600` + `SO_PEERCRED` (mesmo uid) | `debugsock.py:146-157,180` → `debugsock.rs` | feito |
| L12 | Não apagar arquivo alheio; socket órfão só sai com `ECONNREFUSED` | `debugsock.py:122-144` | feito: arquivo que não é socket nunca é apagado (erro `caminho existe e não é socket`) e o órfão só sai quando o `connect` recusa |
| L13 | Erro sempre `{"ok":false,"err":...}`, nunca derruba o app | `debugsock.py:70-119` → `debugsock.rs` | feito |
| L15 | `--dump-to`/`SIGUSR1` para inspeção sem socket | `window.py:947-949` | feito: `--dump-to` e `SIGUSR1` escrevem o dump sem socket |

## 13. Plataforma e distribuição

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| M1 | Um binário sem stack de sistema (sem GTK/Pango/fontconfig/Python) | impossível no Python | feito |
| M2 | pty POSIX por uma API só (`PtySize`, `spawn`, `read`, `write`) | `pty.fork` (POSIX só) → `pty.rs` | feito |
| M3 | Linux X11 + Vulkan | ambiente de dev | feito |
| M5 | Linux backend GL (`WGPU_BACKEND=gl`) | n/a | feito: `WGPU_BACKEND=gl` desenha com GPU real (`Gl AMD Radeon Graphics (radeonsi …)`, `last_frame: drawn`, texto e cursor na captura). Três consertos que só aparecem no GL: display handle na instância (sem ele o EGL responde `gl not compatible with provided surface`), `CompositeAlphaMode::Opaque` (o default dava janela translúcida) e escolha de adaptador preferindo hardware |
| M8 | CI matriz de build + release por tag | não existe → `.github/workflows/release.yml` + `tools/release.sh` | feito para Linux x86_64/aarch64 (os alvos macOS/Windows são `M-X`/`M-Y` do [`BACKLOG`](BACKLOG.md)) |
| M10 | Instalador: comando `bzmux`, menu, ícone, `--uninstall`, `--check` | `install.sh` (pipx + deps de sistema) → `install.sh` (copia 1 binário) | feito |

## 14. Testes e verificação

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| N1 | Suíte de checks do núcleo sem display (`checks.py`, 27 checks) | `checks.py:820-838` → `checks.rs`/`checks_n2.rs`/`checks_n5.rs` + `checks_app.rs` | feito: **31 checks** no `--selftest` (25 do núcleo + 6 do lado do app: `argv_parsing`, `kitty_keyboard`, `mouse_modes_and_sgr`, `window_state`, `debug_socket`, `desktop_shortcut`), e o `cargo test` roda a mesma lista (`selftest_completo`/`checks_do_app_passam`) |
| N2 | Checks de config/CLI | `checks.py:125-287` | feito: `config_file`, `config_options` e `scrollback_and_bell` no `--selftest` (formato do kitty, merge com padrões, erro por linha com motivo, teto de histórico, RIS não é dono da config, BEL só conta) + 7 testes de argv e do merge CLI > arquivo > padrão em `cli.rs` |
| N3 | Checks de teclado/mouse | `checks.py:325-343,554-562` | feito: 6 testes de kitty keyboard (push/pop/set/query, flags 1/2/8/16, `:2`/`:3`, DECCKM, texto da flag 16) e 3 de mouse (modos 1000/1002/1003/1005/1006, SGR vs X10, roda e modificadores) |
| N4 | Checks de socket | `checks.py:702-810` | feito: `DebugServer` aceita um `Sink` (o `EventLoopProxy` no app, um canal no teste), então 8 testes rodam sem janela: framing de uma linha JSON, `ok`/`err`, vários pedidos na mesma conexão e num `write` só, `0600`, órfão vs socket vivo vs arquivo que não é socket, `resolve("auto")` |
| N5 | Checks de reflow/seleção/âncora | `checks.py:380-520` | feito: `history`, `scroll_anchor`, `redraw_keeps_history_position`, `draw_damage`, `selection`, `truecolor` e `style_contrast` no `--selftest` (`paste_markers` e `selection_follows_history` já existiam) |
| N6 | `--selftest` no binário (roda N1..N5 sem display) | `cli.py:92` → `checks.rs` + `checks_app.rs` | feito: `bzmux --selftest` (**31 checks**), no `check.sh` e no CI |
| N7 | Verificação visual automatizada (captura + comparação) | `tools/pixel.py`, `xtest.py` → `tools/pixel.py`, `tools/visual.sh` | feito: `tools/visual.sh` abre a janela num `Xvfb` próprio, espera o primeiro frame, captura e **compara** — conteúdo novo tem de mudar pixel, SGR 41/42 tem de pintar `color1`/`color2`, texto tem de cair dentro da célula e tela parada não pode repintar diferente. Sai 0/1, então serve de gate |
| N8 | Benchmark reprodutível com grade travada | `tools/benchrun.py` → `tools/benchrun.py` | feito |
| N9 | Escada Rust x Python | `bench.sh` → `tools/bench.sh` | feito |

---

## 15. Contagem por área

Não existe tabela aqui: o painel no topo deste arquivo é a **única** contagem.
Uma segunda cópia envelhece calada (esta existiu e ficou 9 itens atrás).

## 16. A ordem que a migração seguiu

A ordem não foi a das tabelas: foi por **o que destrava uso diário** e por
dependência. Fechou assim, cada fatia com o critério que a fechou:

| fatia | o que era | o critério que valeu |
|---|---|---|
| F0 | higiene: `fmt`, `clippy`, um comando só no hook e no CI | `./tools/check.sh` verde do zero |
| F1 | terminal utilizável antes de qualquer cromo | `vim` e `mc` com mouse, cópia entre duas instâncias, `bat` com itálico/negrito, `dmesg` navegável por teclado |
| F2 | reflow e âncoras | estreitar a janela com histórico longo preserva o parágrafo e a seleção; `CSI 3J` não pula a visão |
| F3 | configuração em arquivo, com recarga a quente | fonte/cor/padding/bell/scrollback mudam ao vivo; linha inválida **mantém** a config anterior |
| F4 | abas, workspaces e cromo | atalhos, cartões, renomear, arrastar para reordenar, roda na barra, título por OSC, flash de bell |
| F5 | protocolos de entrada (kitty keyboard) | flags 1/2/8/16 conferidas no pty, não no papel |
| F6 | imagens do kitty | `icat` desenha, rola junto e é podada do histórico |
| F7 | pisca de cursor (o resto virou atividade nova) | A/B de frame com o cursor piscando e com SGR 5 |
| F8 | portas Linux | `WGPU_BACKEND=gl` desenha com GPU real (`radeonsi`) |
| F9 | splits | não é paridade: foi para o `BACKLOG` antes de começar |

**Daqui para frente a fila é o [`BACKLOG.md`](BACKLOG.md)**, e ela é de atividade
nova: paridade pendente não existe mais.

### 16.1 Regras que valeram no caminho

Ficam registradas porque explicam o formato das linhas deste arquivo: fatia só
começava com `cargo test` verde e os roteiros manuais dela passando; refatoração
que não era requisito virava `BACKLOG`, não entrava na fatia; e toda
funcionalidade migrada trazia o teste equivalente do `checks.py` — quando não
havia equivalente, o roteiro manual ficava escrito na própria linha.

### 16.2 Checks: onde moram e o que falta

Os checks de fumaça rodam no `--selftest` (e no `cargo test`, pela mesma lista):
**25 no núcleo** (`checks.rs`, `checks_n2.rs`, `checks_n5.rs` — parser, grade,
scrollback, seleção, dano, config, gráficos) e **6 no lado do app**
(`checks_app.rs` — `argv_parsing`, `kitty_keyboard`, `mouse_modes_and_sgr`,
`window_state`, `debug_socket`, `desktop_shortcut`). O núcleo não conhece
`cli.rs`/`keys.rs`/`mouse.rs`/`state.rs`/`debugsock.rs`, e é por isso que existem
os dois donos: o `checks.py` do Python cobria os 31 num arquivo só.

Três checks do Python não têm nome igual aqui, porque a cobertura é a mesma com
outro recorte: `graphics_image_and_offset` + `graphics_follow_history` viraram
`graphics_kitty` (offset e histórico num teste só, sobre as mesmas imagens), e
`shortcut_remove_only_own_name` virou `desktop_shortcut` (instala, confere o
`.desktop`/ícone e remove no `--uninstall` — o "só apaga o que é seu" do Python
é a metade da remoção).

Check novo entra na lista e **compara com o esperado** — "não panicou" não é
check. O que exige janela não é check: é [`tools/visual.sh`](../tools/visual.sh).
