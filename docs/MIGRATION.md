# Migração: bzmux Python → bzmux Rust

Painel de controle da migração. Uma linha por funcionalidade do bzmux Python,
com o estado dela aqui. É por este arquivo que se responde "quanto falta" — e
não por estimativa.

- **O Python é a especificação observável** (`~/tortty-python`): o que
  o app faz hoje, com `arquivo:linha` em cada linha das tabelas abaixo.
- **O Rust é o destino.** Tudo que existe lá e não existe aqui é `falta`, não
  limite. O que for decisão de **não** fazer mora em
  [`LIMITATIONS.md`](LIMITATIONS.md) e aqui aparece como `não`.
- Número de desempenho não entra aqui: [`BENCHMARKS.md`](BENCHMARKS.md).
- Registro datado do que já foi fechado: [`HISTORY.md`](HISTORY.md).

## Painel
| área | feito | parcial | falta | não | total |
|---|---|---|---|---|---|
| 1. Núcleo: célula, cores, grade | 22 | 0 | 2 | 0 | 24 |
| 2. Parser: controles, CSI, modos | 32 | 3 | 1 | 0 | 36 |
| 3. Parser: SGR e OSC | 14 | 0 | 5 | 0 | 19 |
| 4. Scrollback e visão do histórico | 9 | 0 | 2 | 0 | 11 |
| 5. Entrada: teclado e protocolo do kitty | 8 | 0 | 12 | 0 | 20 |
| 6. Entrada: mouse, seleção, clipboard, paste | 14 | 1 | 5 | 0 | 20 |
| 7. Desenho: fonte, geometria, cores | 12 | 2 | 3 | 0 | 17 |
| 8. Imagens do protocolo kitty | 0 | 0 | 16 | 3 | 19 |
| 9. Janela: abas, workspaces, cromo | 9 | 2 | 9 | 0 | 20 |
| 10. Janela: ciclo de vida, geometria, estado | 3 | 0 | 8 | 0 | 11 |
| 11. Configuração e CLI | 17 | 1 | 3 | 0 | 21 |
| 12. Debug socket e instrumentação | 7 | 5 | 3 | 0 | 15 |
| 13. Plataforma e distribuição | 5 | 0 | 5 | 0 | 10 |
| 14. Testes e verificação | 3 | 2 | 4 | 0 | 9 |
| **total** | **155** | **16** | **78** | **3** | **252** |

Equivalente pronto: **64.7%** (feito conta 1, parcial conta 0,5). O total serve para ver
tendência; o que decide o que fazer agora é a linha e a seção 17.

## Regras deste documento

1. **Uma linha, uma funcionalidade.** Se a linha precisa de "e" para descrever,
   são duas linhas.
2. **Estado sempre preenchido** com um destes: `feito`, `parcial`, `falta`,
   `não`. `parcial` exige dizer **o que falta** na própria linha.
3. **Fechar item** = mudar o estado, rodar a verificação da coluna
   "verificação" e registrar uma linha em `HISTORY.md` com a data. O item não
   sai daqui: o painel conta o que está feito, não o que sobrou.
4. **Item novo** (descoberto no meio do caminho) entra como linha nova na área,
   não como parágrafo. Se o Python não tem, é **melhoria** (seção 16), não
   migração.
5. **Ordem de execução** não mora nas tabelas: mora na seção 17. Marcar `feito`
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
| A10 | Charset G1 + SO/SI (lock shift) | não existe no Python → `term.rs` | feito (**melhoria**) |
| A11 | Wide char (`wcwidth`) com célula de continuação | não existe (`LIMITATIONS.md:22-27`) → `cell.rs`, `term.rs` | feito (**melhoria**) |
| A12 | Combinantes/grafema: compor na célula anterior em vez de ocupar coluna | não existe (`LIMITATIONS.md:29-33`) | falta |
| A13 | Tab stops configuráveis (HTS/TBC/CHT/CBT) | tab fixo de 8 (`term.py:423-426`) → `term.rs` | feito (**melhoria**) |
| A14 | Origin mode `?6` | não existe → `term.rs` | feito (**melhoria**) |
| A15 | Autowrap desligável `?7` | ignorado (`LIMITATIONS.md:57`) → `term.rs` | feito (**melhoria**) |
| A16 | Dano por linha (`dirty`/`dirty_all`) | `term.py:355-362` → `term.rs` | feito |
| A17 | Render usa o dano (só remonta as linhas sujas) | `view.py:920-935` → — | falta |
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
| B3 | C1 (0x80-0x9F) | desenha como texto (`term.py:862-866`) | parcial: UTF-8 os transforma em U+0080..U+009F imprimíveis |
| B4 | `ESC [ ` CSI, `ESC ]` OSC, `ESC P/_/^/X` DCS/APC/PM/SOS | `term.py:871-876` → `term.rs` | feito |
| B5 | DCS/APC/PM/SOS consumidos até ST (sem vazar na tela) | **vaza** no Python (`term.py:895-896`) → `term.rs` | feito (**melhoria**) |
| B6 | `ESC 7`/`ESC 8` DECSC/DECRC | `term.py:882-888` → `term.rs` | feito |
| B7 | DECSC salva também SGR/origin/autowrap | Python salva só cursor → `term.rs` | feito (**melhoria**) |
| B8 | `ESC D` IND / `ESC E` NEL | no-op no Python (`term.py:891-892`) → `term.rs` | feito (**melhoria**) |
| B9 | `ESC M` RI (scroll reverso na margem) | só move cursor (`term.py:879-881`) → `term.rs` | feito |
| B10 | `ESC H` HTS, `ESC c` RIS | RIS `term.py:889-890` → `term.rs` | feito |
| B11 | `ESC # 8` DECALN | não existe | falta |
| B12 | CSI cursor: CUU/CUD/CUF/CUB/CNL/CPL/CHA/VPA/CUP/HVP | `term.py:688-703` → `term.rs` | feito |
| B13 | CSI `s`/`u` (SCOSC/SCORC) e `u` com prefixo = kitty kbd | `term.py:734-740` → `term.rs` | parcial: `s`/`u` feitos; kitty kbd falta (D-seção 5) |
| B14 | IL/DL/ICH/DCH/ECH | `term.py:708-728` → `term.rs` | feito |
| B15 | Clamp de `n` em IL/DL/ICH/DCH/ECH (o Python não clampa) | bug `term.py:483-497,712-728` → `term.rs` | feito (**melhoria**) |
| B16 | EL 0/1/2 e ED 0/1/2 | `term.py:704-707` → `term.rs` | feito |
| B17 | ED 3 limpa scrollback + invalida âncoras | `term.py:456-465` → `term.rs` | feito |
| B18 | SU/SD | não existem no Python | feito (**melhoria**) |
| B19 | DECSTBM (região de scroll) | `term.py:729-733` → `term.rs` | feito |
| B20 | REP (`CSI b`) | não existe (`LIMITATIONS.md:59`) → `term.rs` | feito (**melhoria**) |
| B21 | Modos privados `?1` DECCKM, `?25` DECTCEM | `term.py:594-601` → `term.rs` | feito |
| B22 | `?47`/`?1047`/`?1049` alt screen (+ limpeza de histórico no 1049) | `term.py:602-629` → `term.rs` | feito |
| B23 | `?1048` save/restore cursor | `term.py:630-634` → `term.rs` | feito |
| B24 | `?1000`/`?1002`/`?1003` (estado do mouse) | `term.py:635-639` → `term.rs` | feito (estado; a codificação falta — E) |
| B25 | `?1006` SGR mouse (estado) | `term.py:640-641` → `term.rs` | feito (estado; codificação falta) |
| B26 | `?1004` focus report (estado + envio no app) | `term.py:642-643` → `term.rs`, `app.rs` | feito |
| B27 | `?2004` bracketed paste + payload `200~`/`201~` | `term.py:648-651,1082-1091` → `select.rs` | feito |
| B28 | `?2026` synchronized output | `term.py:652-655` | parcial: estado existe, o frame não é segurado |
| B29 | DSR/CPR `CSI 6 n`, status `CSI 5 n` | `term.py:772-773` → `term.rs` | feito |
| B30 | DA1 `CSI c`, DA2 `CSI > c` | `term.py:743-749` → `term.rs` | feito |
| B31 | DECRQM `CSI ? … $ p` | `term.py:750-754` → `term.rs` | feito |
| B32 | `CSI 16 t` (célula) e `CSI 18 t` (grade) | `term.py:755-762` → `term.rs` | feito |
| B33 | DECSCUSR `CSI Ps SP q` (+ volta à config no 0) | `term.py:764-771` → `term.rs`, `render.rs` | feito |
| B34 | Buffer de resposta ao filho (`child_out`) | `term.py:140` → `term.rs` | feito |
| B35 | UTF-8 incremental com parcial bufferizado e inválido → U+FFFD | `view.py:227,404-406` → `term.rs` | feito |
| B36 | Estado `csi`/`osc` com teto de comprimento | Python concatena sem limite (`term.py:906,919,932`) → `term.rs` | feito (**melhoria**) |

## 3. Parser: SGR e OSC

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| C1 | SGR 0, 1, 4, 7, 22, 24, 27 | `term.py:514-527` → `term.rs` | feito |
| C2 | SGR 30-37, 39, 40-47, 49, 90-97, 100-107 | `term.py:528-539` → `term.rs` | feito |
| C3 | SGR 38;5 / 48;5 (256) e 38;2 / 48;2 (truecolor) | `term.py:540-576` → `term.rs` | feito |
| C4 | Subparâmetros `:` (`38:2::r:g:b`) | `term.py:500-512,579-591` → `term.rs` | feito |
| C5 | SGR 3 itálico, 2 faint, 5/6 blink, 9/29 riscado | não existem (`LIMITATIONS.md:44-53`) → `cell.rs`, `render.rs` | feito (SGR 5 anima: fase de 500 ms em `Blink`) |
| C6 | SGR 53/55 overline, 58/59 cor do sublinhado | não existem → `term.rs` | feito |
| C7 | SGR 4:2/4:3/4:4/4:5 (estilos de sublinhado) | `4:x` colapsa em simples (`term.py:518-519`) → `render.rs` | feito (curvo é aproximação em 4 retângulos) |
| C8 | Bold sem "bold as bright" | `view.py:992` → `render.rs` | feito (mesma semântica) |
| C9 | OSC 0/2 título | `term.py:959-960` → `term.rs` | feito |
| C10 | OSC 1 (título de ícone) | ignorado no Python | feito (ignorado, igual) |
| C11 | OSC 10/11 query de cor default | `term.py:961-965` → `Term::osc_color` | feito |
| C12 | OSC 4 query de paleta | `term.py:966-980` → `Term::osc_palette` | feito |
| C13 | OSC 4/104/110/111/112 **set** (redefinir cor em runtime) | não existe (`LIMITATIONS.md:66-68`) → `Term::theme`/`theme_dirty` | feito (**melhoria**) |
| C14 | OSC 8 hyperlink (guardar por célula + clique) | consumido e descartado (`LIMITATIONS.md:73-75`) | falta (**melhoria**) |
| C15 | OSC 52 clipboard (com trava de config) | não existe | falta (**melhoria**) |
| C16 | OSC 7 cwd do shell | não existe | falta (**melhoria**) |
| C17 | OSC 133 marcas de prompt (A/B/C/D) | não existe | falta (**melhoria**) |
| C18 | OSC 9/777 notificação | não existe | falta |
| C19 | OSC com terminador BEL **e** ST, ESC solto volta a reprocessar | `term.py:914-926` → `term.rs` | feito |

## 4. Scrollback e visão do histórico

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| D1 | Scrollback com teto configurável (default 1000) | `term.py:109`, `config.py:150` → `term.rs` | feito |
| D2 | Flag de quebra mole paralela ao histórico | `term.py:106-118` → `grid.rs` | feito |
| D3 | `visible_rows(off)` sempre devolve `rows` linhas | `term.py:1006-1024` → `term.rs` | feito |
| D4 | Roda do mouse rola 3 linhas | `view.py:1208-1226` → `app.rs` | feito |
| D5 | Roda com delta fracionário (trackpad) acumula | não existe (GDK smooth é descartado) → `app.rs` | feito (**melhoria**) |
| D6 | Shift+PgUp/PgDn/Home/End | `view.py:1285-1292` → `app.rs` | feito |
| D7 | Qualquer tecla volta ao vivo (modificador não conta) | `view.py:1290-1292` → `app.rs` | feito |
| D8 | Indicador `↑N` no canto quando no histórico | `view.py:1082-1091` → `render.rs` | feito |
| D9 | Barra de rolagem (visual, arrastável) | não existe no Python | falta (**melhoria**) |
| D10 | Visão fixada ao ler enquanto chega output | `view.py` + `term.py:1026-1078` → `Term::view_after_output` | feito |
| D11 | Busca no scrollback | não existe | falta (**melhoria**) |

## 5. Entrada: teclado e protocolo do kitty

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| E1 | Tabela de teclas simples (setas, Home/End, Ins/Del, PgUp/PgDn, F1-F12) | `keys.py:46-74` → `keys.rs` | feito |
| E2 | Modificador como parâmetro (`CSI 1;mA`) | `keys.py:116-138` → `keys.rs` | feito |
| E3 | DECCKM: setas/Home/End em modo aplicação (`ESC O A`) | `keys.py:78-84` → `keys.rs` | feito |
| E4 | Shift+Tab (`ISO_Left_Tab` → `CSI Z`) | `keys.py:180-181` → `keys.rs` | feito |
| E5 | Ctrl+letra (`chr(kv&0x1f)`), Ctrl+Space, Ctrl+minus | `keys.py:192-199` → `keys.rs` | feito (Ctrl+Space = NUL, Ctrl+\ = 0x1c) |
| E6 | Alt como prefixo ESC em tecla especial e em texto | `keys.py:121-138,215-216` → `keys.rs` | feito (tecla especial vira `CSI 1;3x`, texto vira `ESC` + letra) |
| E7 | F13-F24 | ausentes no Python também | falta (**melhoria**) |
| E8 | Teclado numérico com NumLock off (`KP_*`) | ausente no Python (`LIMITATIONS.md:87`) | falta (**melhoria**) |
| E9 | Dead keys / composição (ABNT2 `~ ^ ´`) | `keys.py:13-44,219-227` → winit | feito (o winit entrega o texto composto; `Key::Dead` não gera byte) |
| E10 | Texto imprimível vindo do evento (layout não-US) | `view.py:1295-1300` → `app.rs` | feito (usa `event.text` e cai em `acquired` quando vazio) |
| E11 | IME / preedit (CJK, compose) | não existe no Python | falta (**melhoria**) |
| E12 | Kitty kbd: `CSI > u` push, `CSI < n u` pop, `CSI = f;m u` set | `term.py:982-1001` | falta |
| E13 | Kitty kbd: `CSI ? u` query | `term.py:986` | falta |
| E14 | Kitty kbd flag 1 (disambiguate) | `kitty_kbd.py:25-52,115-119` | falta |
| E15 | Kitty kbd flag 2 (event types, release `:3`) | `kitty_kbd.py:90-109` | falta |
| E16 | Kitty kbd flag 4 (alternate keys) | ausente no Python (`LIMITATIONS.md:86`) | falta (**melhoria**) |
| E17 | Kitty kbd flag 8 (all keys as CSI u) | `kitty_kbd.py:127-135` | falta |
| E18 | Kitty kbd flag 16 (associated text) | `kitty_kbd.py:62-79` | falta |
| E19 | Release sem forma kitty não manda texto (bug do `pi`) | `kitty_kbd.py:90-99` | falta (depende de E12-E18) |
| E20 | Atalhos locais têm precedência sobre o encoder | `view.py:1263-1267` | falta (não há atalhos locais ainda) |

## 6. Entrada: mouse, seleção, clipboard, paste

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| F1 | Encoding X10 legado (`CSI M` + 3 bytes, release=3, coord ≤222) | `mouse.py:57-64` → `mouse.rs` | feito |
| F2 | Encoding SGR (`CSI < b;x;y M/m`) | `mouse.py:51-64` → `mouse.rs` | feito |
| F3 | Modificadores no código do botão (+4/+8/+16) | `mouse.py:26,34-36` → `mouse.rs` | feito |
| F4 | Roda como botões 64-67 (press e release no SGR) | `mouse.py:21-23,46-47` → `mouse.rs` | feito |
| F5 | Botões 8/9 (back/forward) | `mouse.py:22,30` → `mouse.rs` | feito |
| F6 | Motion com botão (`?1002`) e sem botão (`?1003`) | `view.py:1199-1205` → `app.rs` | feito |
| F7 | Mouse reporting em pixels (`?1016`) | não existe | falta (**melhoria**) |
| F20 | `?1005` coordenada em UTF-8 (sai do teto de 222) | Python ignora (`term.py:644`) | feito (**melhoria**) |
| F8 | Seleção por arrasto com limiar de 3 px | `view.py:1145-1200` → `app.rs` | feito |
| F9 | Seleção em âncora absoluta (sobrevive a rolagem e output) | `view.py:854-881`, `term.py:1056-1075` → `select.rs` | feito (o reflow da A21 ainda mata a âncora, igual ao Python) |
| F10 | Seleção por palavra (duplo clique) e por linha (triplo) | não existe (`LIMITATIONS.md:71-72`) | falta (**melhoria**) |
| F11 | Seleção retangular / bloco | não existe | falta (**melhoria**) |
| F12 | Seleção: cópia no release para PRIMARY; `copy_on_select` copia CLIPBOARD | `view.py:1168-1176` → `app.rs` | parcial: copia para PRIMARY **e** CLIPBOARD; `copy_on_select` vira opção na F3 |
| F13 | Ctrl+Shift+C copia (seleção ou viewport), Ctrl+Shift+V cola | `view.py:1271-1277,1239-1254` → `app.rs` | feito |
| F14 | Botão 2 cola PRIMARY | `view.py:1152-1154` → `clip.rs` | feito |
| F15 | Bracketed paste envolve payload em `200~`/`201~` | `term.py:1082-1091` → `select.rs` | feito |
| F16 | Clipboard sobrevive ao exit / integração com o sistema | `Gtk.Clipboard` → `clip.rs` (`arboard`) | feito no CLIPBOARD; PRIMARY morre com o app (é o dono da seleção) |
| F17 | Abrir link com Ctrl+clique | não existe | falta (depende de C14) |
| F18 | Menu de contexto | não existe | falta |
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
| G7 | Fallback para emoji/CJK | `HISTORY.md` P10 | parcial: usa fallback do `fontdb`; sem emoji colorido |
| G8 | Ligaduras/shaping | Pango (P9) | falta: `swash` sem shaping complexo |
| G9 | Padding da janela em px | `config.py` `window_padding_width` → shader (`origin`) | feito |
| G10 | Fundo da sobra de célula até a borda (sem tarja preta) | `view.py:926-934` | feito (fundo pintado por instância de grade inteira) |
| G11 | Paleta/tema configurável | não existe (`LIMITATIONS.md:133-140`) → `Theme` | feito (**melhoria**) |
| G12 | Cursor: block/beam/underline + pisca | `view.py:1058-1079`, `828-838` → `render.rs`, `app.rs` | feito: `cursor_blink on` pisca a 500 ms, tecla/colagem reacende, `CSI 6 q` par vence; SGR 5 usa a mesma fase |
| G13 | Sublinhado na largura do run | `view.py:1022-1025` → `render.rs` | feito |
| G14 | Seleção desenhada como inversão | `view.py:943,953-973` → `render.rs` | feito (o default resolve pelo outro extremo) |
| G15 | `?2026` segura o frame (teto 200 ms) | `view.py:296-324` | falta |
| G16 | Frame coalescido (~16 ms), não um desenho por leitura | `view.py:286-320` | parcial: desenha por leitura quando `--no-vsync` |
| G17 | Dano por região (`queue_draw_area`) | `view.py:330-364` | falta |

## 8. Imagens do protocolo kitty

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| H1 | APC `ESC _ G … ST` roteado para o subsistema | `term.py:944-953` | falta |
| H2 | `a=t` transmitir / `a=T` transmitir+exibir | `graphics.py:188-212` | falta |
| H3 | `a=p` exibir imagem já transmitida | `graphics.py:184-192` | falta |
| H4 | `a=d` apagar (por id e tudo) | `graphics.py:175-183` | falta |
| H5 | `a=q` query de capacidade | `graphics.py:173-181` | falta |
| H6 | Chunking `m=1` | `graphics.py:146-155` | falta |
| H7 | `f=24/32` (RGB/RGBA cru) e `f=100` (PNG) | `graphics.py:190-196` | falta |
| H8 | `o=z` (zlib) | `graphics.py:161-168` | falta |
| H9 | `s=`/`v=`, `i=` automático, `c=`/`r=` default, `X=`/`Y=` offset px | `graphics.py:100-106,193-207` | falta |
| H10 | `C=1` não move o cursor; senão desce `rows` | `graphics.py:184-187` | falta |
| H11 | `q=` silencia resposta | `graphics.py:170` | falta |
| H12 | Imagem acompanha a rolagem e é podada do histórico | `graphics.py:74-96` | falta |
| H13 | `ED 3` limpa imagens | `term.py:481-485` | falta |
| H14 | z-index (`z=`) e placement id (`p=`) | não existem no Python | falta (**melhoria**) |
| H15 | Unicode placeholder (`U=1`) | não existe | falta (**melhoria**) |
| H16 | Origem recortada (`x`,`y`,`w`,`h`) | não existe | falta (**melhoria**) |
| H17 | `t=f`/`t=t`/`t=s` (arquivo, temp, shm) | não existem (`LIMITATIONS.md:99-108`) | não (decisão: só base64) |
| H18 | Animação (`a=f`/`a=a`) | ignorado de propósito | não |
| H19 | Sixel / iTerm2 inline | não existem | não |

## 9. Janela: abas, workspaces, cromo

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| I1 | Aba = termo + pty com ciclo de vida próprio | `window.py:140-211` → `app.rs:Tab` | feito: `Tab` tem term/pty/rx/visão/seleção; `drain` lê todas |
| I2 | Nova aba herda cwd da aba corrente (último descendente do pty) | `window.py:140-153,904-919` | feito: `/proc/<pid>/cwd` do descendente mais fundo; `cd /tmp` → aba nova em `/tmp` |
| I3 | Fechar aba (fecha pty, remove página, escolhe a próxima) | `window.py:184-201` | feito: `Ctrl+Shift+W`; aba de shell morto fecha sozinha |
| I4 | Título da aba por OSC 0/2, senão `basename(cwd)`, senão `bzmux` | `window.py:277-286` | feito: `Tab::title` (OSC → cwd → `bzmux`) |
| I5 | Workspace = conjunto de abas, com nome e renumeração | `window.py:798-848` | falta |
| I6 | Sidebar com cartões (`n`, nome, título, k abas) | `window.py:289-301` | falta |
| I7 | Sidebar escondida com 1 workspace | `window.py:396-400` | falta |
| I8 | Renomear workspace inline (duplo clique, Enter/Esc/focus-out) | `window.py:315-356` | falta |
| I9 | Clique do meio fecha aba (no rótulo) e workspace (no cartão) | `window.py:208-212,307-309` | falta |
| I10 | Arrastar aba para reordenar | `window.py:173` | falta |
| I11 | Roda do mouse na barra de abas | `window.py:100` | falta |
| I12 | Toast (erro/aviso) com fila e timeout | `window.py:411-465,677-740` → `render.rs:toast` | feito: faixa invertida na 1ª linha, 3,5 s, mensagem igual seguida vira `(×N)` |
| I13 | Bell visual: flash do rótulo por 800 ms (800 ms, `BELL_BG`) | `window.py:255-275` → `render.rs:tabbar` | feito: `color3` por 800 ms, inclusive em aba de segundo plano |
| I14 | Bell audível (`beep`) | `window.py:666-669` | feito (`bell audible` escreve BEL no stdout; sem `beep` de toolkit) |
| I15 | Atalhos de aba/workspace (Ctrl+Shift+T/W/N/Q/F2, Alt+N, Super+N/Q) | `window.py:854-898` | parcial: Ctrl+Shift+T/W e Alt+1..9 feitos; falta Ctrl+Shift+N/Q/F2 e Super (F4b) |
| I16 | Dígito de atalho por keycode cru (sobrevive a ABNT2) | `window.py:70-84` → `digit_of` (tecla física) | feito |
| I17 | Título da janela = título da aba ativa | `window.py:375,921-926` | feito: `<título> — bzmux`, só quando muda |
| I18 | Abas/workspaces persistentes entre execuções | não existem (`LIMITATIONS.md:141`) | falta (**melhoria**) |
| I19 | Splits (mais de um terminal na mesma view) | não existe (P39) | falta (**melhoria**) |
| I20 | Fechar última aba fecha o workspace; último workspace fecha o app | `window.py:196-197,836-845` | parcial: última aba fecha o app; falta o nível de workspace |

## 10. Janela: ciclo de vida, geometria, estado

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| J1 | Fecha a janela quando o shell termina | `window.py:250-253` → `app.rs` | feito |
| J2 | Geometria inicial 980×640 e restauração de tamanho | `window.py:24-31,467-478` | falta |
| J3 | Persistência de geometria (`$XDG_STATE_HOME/bzmux/window.json`) | `state.py:16-63` | falta |
| J4 | Rejeitar geometria inválida (monitor desconectado, lixo) | `state.py:46-63` | falta |
| J5 | Maximizado é estado separado do tamanho normal | `window.py:1009-1012` | falta |
| J6 | Wayland: não salvar x/y | `window.py:975-978` | falta |
| J7 | `SIGUSR1` = dump imediato; `SIGTERM` = sair limpo | `window.py:503-508` | falta |
| J8 | `--quit-after SEG` (usado pela automação) | `cli.py:35` | falta |
| J9 | Dump do texto de todas as abas para arquivo | `window.py:929-945` | falta |
| J10 | Grid travado pelo socket sobrevive a arredondamento do WM | não existe (`benchrun.py` depende de 6 tentativas) → `app.rs` | feito (**melhoria**) |
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
| K15 | `--quit-after`, `--dump-to`, `--selftest`, `--install-shortcut` | `cli.py:35-96` | parcial: `--selftest` feito (atalho virou o `install.sh`); falta `--quit-after`/`--dump-to` |
| K16 | `--version`, `--help`, código 2 para uso inválido | `cli.py:85-101` → `cli.rs` | feito |
| K17 | `--cols`/`--rows`/`--dpi`/`--no-vsync`/`--prof` | não existem no Python → `cli.rs` | feito (**melhoria**) |
| K18 | `.desktop` + ícone (`--install-shortcut`) | `shortcut.py:34-136` | falta |
| K19 | WM_CLASS estável (`StartupWMClass`) | `cli.py:130-131` | falta |
| K20 | Remapeamento de atalhos | não existe | falta (**melhoria**, precisa de atalhos locais — F4) |
| K21 | Paleta no arquivo de config (`color0..15`, fg/bg/cursor) | não existe (`LIMITATIONS.md:133-140`) → `config.rs` | feito (**melhoria**) |

## 12. Debug socket e instrumentação

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| L1 | Transporte Unix socket, JSON por linha, vários pedidos por conexão | `debugsock.py:217-240` → `debugsock.rs` | feito |
| L2 | `state` com grade, cursor, título, histórico, célula, fonte, dpi | `window.py:531-546` → `app.rs` | parcial: falta `pid`, `xid`, `config`, `dpi`, `advance`, `pin` |
| L3 | `text` (`view`/`history`/`screen`, `start`, `count`) | `debugsock.py:86-93` | parcial: `view`/`history`; falta `screen` |
| L4 | `write` (bytes crus no pty) | `debugsock.py:106-111` | feito |
| L5 | `keys` (spec → teclas, passando pelo encoder e atalhos) | `debugsock.py:94-99` → `app.rs` | parcial: usa o encoder real; falta passar pelos atalhos locais (que ainda não existem) |
| L6 | `resize` | `debugsock.py:100-103` | feito |
| L7 | `prof` (contadores por etapa) | `debugsock.py:104-105` | parcial: `draw`/`parse`/`pty`; sem `bytes`/`eagain` |
| L8 | `log` (ligar/desligar log em runtime) | `debugsock.py:112-113` | parcial: escreve; falta `off`/path |
| L9 | `quit` (resposta sai antes de fechar) | `debugsock.py:114-115` | feito |
| L10 | `--debug-socket auto` (`$XDG_RUNTIME_DIR/bzmux-<pid>.sock`) | `debugsock.py:33-43` → `debugsock.rs` | feito |
| L11 | Socket `0600` + `SO_PEERCRED` (mesmo uid) | `debugsock.py:146-157,180` → `debugsock.rs` | feito |
| L12 | Não apagar arquivo alheio; socket órfão só sai com `ECONNREFUSED` | `debugsock.py:122-144` | falta |
| L13 | Erro sempre `{"ok":false,"err":...}`, nunca derruba o app | `debugsock.py:70-119` → `debugsock.rs` | feito |
| L14 | Named pipe no Windows (transporte alternativo) | n/a (Python é POSIX) | falta (**melhoria**) |
| L15 | `--dump-to`/`SIGUSR1` para inspeção sem socket | `window.py:947-949` | falta (ver J7/J9) |

## 13. Plataforma e distribuição

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| M1 | Um binário sem stack de sistema (sem GTK/Pango/fontconfig/Python) | impossível no Python | feito |
| M2 | pty POSIX e ConPTY pela mesma API | `pty.fork` (POSIX só) → `pty.rs` | feito |
| M3 | Linux X11 + Vulkan | ambiente de dev | feito |
| M4 | Linux Wayland | n/a | falta |
| M5 | Linux backend GL (`WGPU_BACKEND=gl`) | n/a | falta |
| M6 | Windows 10/11 (D3D12 + ConPTY) | não roda | falta |
| M7 | macOS arm64 e x86_64 (Metal) | não roda | falta |
| M8 | CI matriz de build + release por tag | não existe → `.github/workflows/release.yml` + `tools/release.sh` | feito para Linux x86_64/aarch64; macOS/Windows entram com M6/M7 |
| M9 | Notarização macOS / assinatura Windows | não existe | falta (decisão adiada: ver `PORTABILITY.md`) |
| M10 | Instalador: comando `bzmux`, menu, ícone, `--uninstall`, `--check` | `install.sh` (pipx + deps de sistema) → `install.sh` (copia 1 binário) | feito |

## 14. Testes e verificação

| # | funcionalidade | ref (Python → Rust) | estado |
|---|---|---|---|
| N1 | Suíte de checks do núcleo sem display (`checks.py`, 27 checks) | `checks.py:820-838` → `checks.rs` + `#[test]` | parcial: 14 checks de fumaça + 52 testes; falta socket/teclado/mouse |
| N2 | Checks de config/CLI | `checks.py:125-287` | falta (depende de K1) |
| N3 | Checks de teclado/mouse | `checks.py:325-343,554-562` | falta (depende de E/F) |
| N4 | Checks de socket | `checks.py:702-810` | falta |
| N5 | Checks de reflow/seleção/âncora | `checks.py:380-520` | falta |
| N6 | `--selftest` no binário (roda N1..N5 sem display) | `cli.py:92` → `checks.rs` | feito: `bzmux --selftest` (14 checks), no `check.sh` e no CI |
| N7 | Verificação visual automatizada (captura + comparação) | `tools/pixel.py`, `xtest.py` → `tools/` | parcial: captura manual (`xwd`) |
| N8 | Benchmark reprodutível com grade travada | `tools/benchrun.py` → `tools/benchrun.py` | feito |
| N9 | Escada Rust x Python | `bench.sh` → `tools/bench.sh` | feito |

---

## 15. Contagem por área

| área | feito | parcial | falta | não |
|---|---|---|---|---|
| Núcleo: célula, cores, grade | 22 | 0 | 2 | 0 |
| Parser: controles, CSI, modos | 32 | 3 | 1 | 0 |
| Parser: SGR e OSC | 14 | 0 | 5 | 0 |
| Scrollback e visão do histórico | 9 | 0 | 2 | 0 |
| Entrada: teclado e protocolo do kitty | 8 | 0 | 12 | 0 |
| Entrada: mouse, seleção, clipboard, paste | 14 | 1 | 5 | 0 |
| Desenho: fonte, geometria, cores | 12 | 2 | 3 | 0 |
| Imagens do protocolo kitty | 0 | 0 | 16 | 3 |
| Janela: abas, workspaces, cromo | 9 | 2 | 9 | 0 |
| Janela: ciclo de vida, geometria, estado | 3 | 0 | 8 | 0 |
| Configuração e CLI | 17 | 1 | 3 | 0 |
| Debug socket e instrumentação | 7 | 5 | 3 | 0 |
| Plataforma e distribuição | 5 | 0 | 5 | 0 |
| Testes e verificação | 3 | 2 | 4 | 0 |
| **total** | **155** | **16** | **78** | **3** |

## 16. Melhorias propostas

O que o Python **não** faz e que esta arquitetura torna barato. Cada uma tem
id, o motivo e o custo estimado; nenhuma é requisito de paridade. As que já
estão implementadas aparecem nas tabelas marcadas `(**melhoria**)`.

| id | melhoria | por que | custo |
|---|---|---|---|
| M-A | Render usar o dano por linha (A17/G17) | hoje monta instância de todas as linhas; a 61×18 custa 0,28 ms, mas em 4K/250×70 o custo passa a ser proporcional à tela, não à atividade | baixo: o core já marca `dirty[]`; falta só filtrar no `build` e manter buffer persistente |
| M-B | SGR completo: itálico, faint, riscado, blink, overline, cor do sublinhado, estilos de sublinhado (C5-C7) | `git diff`, `bat`, `delta`, `lsd` usam 9 e 4:x; no Python o risco simplesmente não aparece | baixo: 1 instância nova por estilo + face itálica no atlas |
| M-C | OSC 8 hyperlink clicável com Ctrl+clique (C14/F17) | links em `dmesg`, compiladores e `ls --hyperlink`; o Python consome e joga fora | baixo: um `u32` por célula + hover |
| M-D | Busca no scrollback com realce (D11) | hoje só se acha rolando; em log grande é a função que falta | médio: índice por linha ou varredura sob demanda |
| M-E | Seleção por palavra/linha e retangular (F10/F11) | copiar um caminho de log hoje exige arrasto preciso | baixo |
| M-F | IME e composição (E11) | o Python não tem `IMContext`; quem digita CJK ou usa compose não consegue | médio: `winit::Ime` + pré-edição na view |
| M-G | Kitty keyboard completo, incluindo flag 4 (E12-E19) | paridade com o que o próprio bzmux promete (kitty/ghostty) e correção do `:2` (repeat) que o Gdk não entrega | médio |
| M-H | Config v2 com paleta e remapeamento de atalhos (K20/K21) | o Python só tem 10 opções e cores fixas no código | baixo-médio: parser pronto, falta o merge de paleta |
| M-I | Abas/workspaces persistentes (I18) | o Python perde o layout ao fechar; com `state.json` já existindo, é JSON a mais | baixo |
| M-J | Splits (I19) | pedido registrado no P39 do Python; aqui cada split é uma view no mesmo `wgpu` | alto |
| M-K | Notificação OSC 9/777 + urgency (C18) | "terminou o build" sem precisar olhar a janela | baixo |
| M-L | Imagens: z-index, placement id, placeholder, recorte de origem (H14-H16) | o Python mostra imagem sempre por cima e sem controle fino | médio |
| M-M | Scrollbar visível e arrastável (D9) | hoje só roda e Shift+Pg (que também falta) | baixo: retângulo + estado |
| M-N | Transporte de socket no Windows (L14) | o `benchrun.py` e a automação precisam funcionar lá também | baixo |
| M-O | `--selftest` no binário (N6) | dá um comando único de verificação sem build de teste, como no Python | feito |

## 17. Ordem de execução

A ordem não é a das tabelas: é por **o que destrava uso diário** e por
dependência. Cada fatia termina com `cargo test` verde, `mc`/`vim`/`dmesg`
funcionando e uma linha em `HISTORY.md`.

**F0 — Higiene (feito, 2026-11-01).** `cargo fmt` + `clippy` (`all`,
`correctness`, `suspicious`, `pedantic`) sem warning, `unsafe_code = deny` com
allow local, um comando único (`tools/check.sh`) no pre-commit e no CI.
Pronto quando: `./tools/check.sh` verde do zero.

**F1 — Terminal utilizável de verdade (antes de qualquer cromo). FEITA em
2026-11-02** — critério conferido: `vim` e `mc` com mouse, cópia/cola entre
duas instâncias, `bat` com itálico/negrito e `dmesg` navegável por teclado.
F8-F9 (seleção), F12-F15 (clipboard/paste), E4-E6 (Ctrl/Alt/Shift+Tab),
E9 (dead keys), F1-F6 (mouse reporting), D6-D8 (scroll por teclado + indicador),
B27 (?2004 payload), C5-C7 (SGR completo), D4-D5 (roda com delta).
Pronto quando: `vim` com mouse, `mc` com mouse, copiar/colar entre apps,
`bat`/`git diff` com riscado, `dmesg` com histórico navegável por teclado.

**F2 — Reflow e âncoras (A21-A23, D10, F9).**
Pronto quando: estreitar a janela com histórico longo preserva o parágrafo e a
seleção acompanha; `CSI 3J` não pula a visão.

**F3 — Configuração (K1-K13, K18-K19, C11-C13, G11, K21). FEITA em
2026-11-02** — critério conferido: `bzmux.conf` muda fonte (14→16 pt ao vivo),
cores (fundo verde→roxo), padding (8→4 px, clique continua na célula certa),
bell, scrollback e `copy_on_select` (on copia PRIMARY+CLIPBOARD, off só
PRIMARY); o auto-reload aplica em ~250 ms e, com linha inválida, **mantém** a
configuração anterior (`linha 3: 'afffont_size': chave desconhecida` no log).
Desvios: erro de config ainda vai para o log, não para toast (K13 depende do
toast da F4); o **pisca** do cursor da config não anima ainda (G12).

**F4 — Abas e workspaces (I1-I17).** O motivo do projeto existir.
Pronto quando: Ctrl+Shift+T/W/N/Q, Alt+N, Super+N/Q, sidebar com cartões,
renomear, flash de bell, título por OSC.

**F5 — Protocolos de entrada completos (E12-E19, F7, C14, F17).**
Kitty keyboard, mouse em pixels, hyperlink clicável.

**F6 — Imagens do kitty (H1-H13).**
Pronto quando `icat` e `kitten icat` desenham, rolam junto e são podadas.

**F7 — Qualidade de vida (D9, D11, F10-F11, G12, E11, C16-C18, M-K).**
Scrollbar, busca, seleção por palavra, cursor piscando, IME, OSC 7/133.

**F8 — Portas (M4-M9, L14).**
Wayland, GL, Windows, macOS, CI matriz, artefatos por tag.

**F9 — Splits (M-J).** Só depois de F4 e F7.

### 17.1 Regras de avanço

- Fatia só começa quando a anterior está com `cargo test` verde e os roteiros
  manuais dela passando. Exceção: item isolado que destrava outro (registrar na
  linha).
- Nada de "aproveitar para refatorar" dentro da fatia: refatoração que não é
  requisito entra como melhoria (seção 16) ou não entra.
- Toda funcionalidade migrada traz o teste equivalente do `checks.py` quando
  existe; quando não existe, o roteiro manual vai escrito na linha.

### 17.2 Checks do Python ainda sem equivalente aqui

`parser_text_and_cursor` (parcial), `sgr_and_erasing` (parcial),
`alt_screen_and_scroll` (parcial), `charset_and_replies` (parcial),
`csi_with_intermediate`, `argv_parsing`, `config_file`, `config_options`,
`scrollback_and_bell`, `kitty_keyboard`, `graphics_image_and_offset`,
`graphics_follow_history`, `wrap_and_control`, `reflow`, `history`,
`scroll_anchor`, `redraw_keeps_history_position`, `selection_follows_history`,
`draw_damage`, `mouse_modes_and_sgr`, `selection`, `paste_markers`,
`truecolor`, `style_contrast`, `desktop_shortcut`, `window_state`,
`debug_socket`.
