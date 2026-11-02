//! Protocolo de gráficos do kitty (APC `ESC _ G … ST`) — a parte pura.
//!
//! Um app manda uma imagem assim::
//!
//! ```text
//! ESC _ G a=T,f=100,i=1,q=2;<base64 do PNG> ESC \
//! ```
//!
//! e o terminal responde (quando não pedem silêncio) `ESC _ G i=1;OK ESC \`.
//! Aqui mora só o parse/decodificação e a lista de *placements*; textura e
//! desenho são do app. Assim o protocolo é testável sem display.
//!
//! Suportado (o suficiente para o `kitten icat`): `a=t`/`a=T`/`a=p`/`a=d`/`a=q`,
//! `f=24` RGB, `f=32` RGBA, `f=100` PNG, `m=1` (imagem em pedaços), `o=z`
//! (zlib), `i=`/`c=`/`r=`/`X=`/`Y=`/`C=`/`q=`.
//!
//! Fora, de propósito: animação (`a=f`/`a=a`), z-index e transmissão por
//! arquivo/temp/shared-memory (`t=f`/`t=t`/`t=s`) — as três estão no
//! `LIMITATIONS.md`.

use std::collections::HashMap;

/// Teto do buffer de uma transmissão em pedaços: app que abre um `m=1` e nunca
/// fecha não pode crescer sem fim (o Python não tem teto; aqui o parser é
/// alimentado por qualquer coisa que escreva no pty).
pub const MAX_APC: usize = 64 * 1024 * 1024;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Formato {
    Rgb,
    Rgba,
    Png,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Image {
    pub id: u32,
    /// Muda a cada transmissão: é como o app sabe que a textura em cache ficou
    /// velha quando o app reusa o mesmo `i=`.
    pub versao: u64,
    pub width: u32,
    pub height: u32,
    pub formato: Formato,
    pub data: Vec<u8>,
}

/// O que está desenhado agora. `linha` é **linha de tela**: 0..rows-1 é a grade
/// viva, e negativo entra no histórico (-1 = última linha do scrollback). Assim
/// rolar é subtrair 1 de todo mundo, que é o que o conteúdo faz.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Placement {
    pub image_id: u32,
    pub col: u16,
    pub linha: i32,
    pub cols: u16,
    pub rows: u16,
    /// `X=`/`Y=`: deslocamento em **pixels** dentro da primeira célula. É como
    /// o `icat` centraliza: ele anda células inteiras e deixa o resto aqui.
    pub dx: i32,
    pub dy: i32,
}

/// O que o `Term` precisa fazer depois de um APC.
#[derive(Default, Debug)]
pub struct Saida {
    /// Corpos de APC a devolver ao app (sem `ESC _` / `ST`).
    pub respostas: Vec<Vec<u8>>,
    /// Quantas linhas a imagem ocupou (o `C=1` pede para não usá-las).
    pub linhas: u16,
    /// O cursor deve descer `linhas`?
    pub mover: bool,
}

#[derive(Default)]
pub struct Graphics {
    images: HashMap<u32, Image>,
    placements: Vec<Placement>,
    next_id: u32,
    /// Contador de versão das imagens (uma por transmissão).
    geracao: u64,
    /// Pedaços de uma transmissão em curso (`m=1`).
    buf: Option<Buf>,
}

struct Buf {
    b64: String,
    kv: HashMap<String, String>,
}

impl Graphics {
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_id: 1,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn images(&self) -> &HashMap<u32, Image> {
        &self.images
    }

    #[must_use]
    pub fn placements(&self) -> &[Placement] {
        &self.placements
    }

    /// `ED 3` (H13): nada de imagem sobrevive.
    pub fn clear(&mut self) {
        self.images.clear();
        self.placements.clear();
        self.buf = None;
    }

    /// Acompanha a rolagem: as colocações sobem junto com o texto.
    ///
    /// `floor`: até que linha (negativa, ou seja dentro do histórico) guardar
    /// uma colocação que saiu pelo topo; `None` = descarta (rolagem de região,
    /// onde não existe histórico). Sem isso a imagem sumia na primeira linha
    /// que rolava e não voltava ao subir no histórico com o wheel.
    pub fn scroll(&mut self, top: u16, bottom: u16, floor: Option<i32>) {
        let top = i32::from(top);
        let bottom = i32::from(bottom);
        self.placements.retain_mut(|p| {
            if p.linha > bottom {
                return true; // fora da região: não mexe
            }
            p.linha -= 1;
            match floor {
                Some(f) => p.linha >= f,
                None => p.linha >= top,
            }
        });
    }

    /// Descarta o que saiu de vez do histórico (as linhas mais antigas).
    pub fn drop_above(&mut self, floor: i32) {
        self.placements.retain(|p| p.linha >= floor);
    }

    fn place(
        &mut self,
        img: &Image,
        kv: &HashMap<String, String>,
        cursor: (u16, u16),
        cell: (u32, u32),
    ) -> u16 {
        let cw = cell.0.max(1);
        let ch = cell.1.max(1);
        let num = |k: &str| kv.get(k).and_then(|v| v.parse::<i64>().ok());
        let cols = num("c").map_or_else(
            || {
                u16::try_from(img.width.div_ceil(cw))
                    .unwrap_or(u16::MAX)
                    .max(1)
            },
            |c| u16::try_from(c.max(1)).unwrap_or(u16::MAX),
        );
        let rows = num("r").map_or_else(
            || {
                u16::try_from(img.height.div_ceil(ch))
                    .unwrap_or(u16::MAX)
                    .max(1)
            },
            |r| u16::try_from(r.max(1)).unwrap_or(u16::MAX),
        );
        self.placements.push(Placement {
            image_id: img.id,
            col: cursor.0,
            linha: i32::from(cursor.1),
            cols,
            rows,
            dx: i32::try_from(num("X").unwrap_or(0)).unwrap_or(0),
            dy: i32::try_from(num("Y").unwrap_or(0)).unwrap_or(0),
        });
        rows
    }

    /// Processa o corpo de um APC (`G…`, já sem os delimitadores).
    pub fn handle(&mut self, body: &[u8], cursor: (u16, u16), cell: (u32, u32)) -> Saida {
        let Some(corpo) = body.strip_prefix(b"G") else {
            return Saida::default();
        };
        let texto = String::from_utf8_lossy(corpo);
        let (ctrl, payload) = texto
            .split_once(';')
            .map_or((texto.as_ref(), ""), |(a, b)| (a, b));
        let mut cur: HashMap<String, String> = HashMap::new();
        for item in ctrl.split(',') {
            if let Some((k, v)) = item.split_once('=') {
                cur.insert(k.to_string(), v.to_string());
            }
        }
        // o control data do **primeiro** pedaço vale para a transmissão
        // inteira; os seguintes só trazem `m=1` (e é do pedaço, não acumula —
        // guardar o `m` fazia a transmissão nunca terminar)
        let novo = self.buf.is_none();
        if novo {
            self.buf = Some(Buf {
                b64: String::new(),
                kv: cur.clone(),
            });
        }
        if let Some(buf) = self.buf.as_mut() {
            if buf.b64.len() + payload.len() > MAX_APC {
                self.buf = None;
                return Saida {
                    respostas: vec![b"Gi=0;EINVAL:too big".to_vec()],
                    ..Saida::default()
                };
            }
            buf.b64.push_str(payload);
        }
        if cur.get("m").is_some_and(|m| m == "1") {
            return Saida::default(); // ainda tem pedaço vindo
        }
        let Some(buf) = self.buf.take() else {
            return Saida::default();
        };
        let kv = buf.kv;
        let Some(mut dados) = base64_decode(&buf.b64) else {
            return Self::erro(&kv, "EINVAL:base64");
        };
        if kv.get("o").is_some_and(|o| o == "z") && !dados.is_empty() {
            // `o=z`: o payload vem comprimido (o `icat --place` manda o PNG
            // decodificado + zlib para escalar sem reencodar)
            match inflate(&dados) {
                Some(d) => dados = d,
                None => return Self::erro(&kv, "EINVAL:zlib"),
            }
        }
        let param = |k: &str| kv.get(k).map(String::as_str);
        let acao = param("a").unwrap_or("t");
        let quieto = param("q").is_some_and(|q| q != "0");
        let ident = param("i").and_then(|i| i.parse::<u32>().ok()).unwrap_or(0);
        self.despacha(acao, &kv, &dados, ident, quieto, cursor, cell)
    }

    /// Roteia a ação do APC. Fica separado do parse para o protocolo caber na
    /// cabeça: primeiro "o que veio", depois "o que fazer".
    #[allow(clippy::too_many_arguments)] // são os campos do pedido, não um saco
    fn despacha(
        &mut self,
        acao: &str,
        kv: &HashMap<String, String>,
        dados: &[u8],
        ident: u32,
        quieto: bool,
        cursor: (u16, u16),
        cell: (u32, u32),
    ) -> Saida {
        let param = |k: &str| kv.get(k).map(String::as_str);
        match acao {
            // animação e controle: ignorados de propósito (H18)
            "f" | "a" | "c" => Saida::default(),
            "q" => {
                // `a=q` é sonda de **capacidade**, não consulta de imagem: o
                // `kitten icat` manda três (direto, arquivo, memória) e só
                // segue se receber OK na transmissão direta. Responder ENOENT
                // por o id não existir faz o icat desistir com "terminal does
                // not support the graphics protocol".
                let meio = param("t").unwrap_or("d");
                let fmt = param("f").and_then(|f| f.parse::<u32>().ok()).unwrap_or(32);
                let ok = meio == "d" && [24, 32, 100].contains(&fmt);
                let nome = if ok {
                    "OK"
                } else {
                    "EINVAL:unsupported transmission"
                };
                Saida {
                    respostas: if ident == 0 {
                        Vec::new()
                    } else {
                        vec![format!("Gi={ident};{nome}").into_bytes()]
                    },
                    ..Saida::default()
                }
            },
            "d" => {
                let como = param("d").unwrap_or("a");
                if como == "i" {
                    self.images.remove(&ident);
                    self.placements.retain(|p| p.image_id != ident);
                } else {
                    self.placements.clear();
                    if como == "a" {
                        self.images.clear();
                    }
                }
                Saida {
                    respostas: if quieto {
                        Vec::new()
                    } else {
                        vec![format!("Gi={ident};OK").into_bytes()]
                    },
                    ..Saida::default()
                }
            },
            "p" => match self.images.get(&ident) {
                None => Saida {
                    respostas: vec![format!("Gi={ident};ENOENT").into_bytes()],
                    ..Saida::default()
                },
                Some(img) => {
                    let img = img.clone();
                    let linhas = self.place(&img, kv, cursor, cell);
                    Saida {
                        respostas: if quieto {
                            Vec::new()
                        } else {
                            vec![format!("Gi={ident};OK").into_bytes()]
                        },
                        linhas,
                        mover: param("C") != Some("1"),
                    }
                },
            },
            "t" | "T" => self.transmite(kv, dados, ident, quieto, cursor, cell),
            _ => {
                if quieto {
                    Saida::default()
                } else {
                    Self::erro(kv, "EINVAL:action")
                }
            },
        }
    }

    fn transmite(
        &mut self,
        kv: &HashMap<String, String>,
        dados: &[u8],
        ident: u32,
        quieto: bool,
        cursor: (u16, u16),
        cell: (u32, u32),
    ) -> Saida {
        let param = |k: &str| kv.get(k).map(String::as_str);
        let mut fmt = param("f").and_then(|f| f.parse::<u32>().ok()).unwrap_or(32);
        if dados.starts_with(b"\x89PNG\r\n\x1a\n") {
            fmt = 100; // o PNG já diz o que é, independente do `f=`
        }
        let (width, height) = if fmt == 100 {
            let (pw, ph) = png_size(dados);
            (
                param("s").and_then(|s| s.parse().ok()).unwrap_or(pw),
                param("v").and_then(|v| v.parse().ok()).unwrap_or(ph),
            )
        } else {
            let w: u32 = param("s").and_then(|s| s.parse().ok()).unwrap_or(0);
            let h: u32 = param("v").and_then(|v| v.parse().ok()).unwrap_or(0);
            let bpp = if fmt == 32 { 4 } else { 3 };
            let precisa = u64::from(w) * u64::from(h) * bpp;
            if w == 0 || h == 0 || (dados.len() as u64) < precisa {
                return Self::erro(kv, "EINVAL:size");
            }
            (w, h)
        };
        if width == 0 || height == 0 {
            return Self::erro(kv, "EINVAL:size");
        }
        let id = if ident == 0 {
            let i = self.next_id.max(1);
            self.next_id = i + 1;
            i
        } else {
            ident
        };
        let formato = match fmt {
            100 => Formato::Png,
            24 => Formato::Rgb,
            _ => Formato::Rgba,
        };
        self.geracao += 1;
        let img = Image {
            id,
            versao: self.geracao,
            width,
            height,
            formato,
            data: dados.to_vec(),
        };
        self.images.insert(id, img.clone());
        let mut out = Saida {
            respostas: if quieto {
                Vec::new()
            } else {
                vec![format!("Gi={id};OK").into_bytes()]
            },
            ..Saida::default()
        };
        if param("a") == Some("T") {
            out.linhas = self.place(&img, kv, cursor, cell);
            out.mover = param("C") != Some("1");
        }
        out
    }

    fn erro(kv: &HashMap<String, String>, msg: &str) -> Saida {
        let ident = kv.get("i").map_or("0", String::as_str);
        Saida {
            respostas: vec![format!("Gi={ident};{msg}").into_bytes()],
            ..Saida::default()
        }
    }
}

/// Largura/altura do IHDR: o PNG já diz, então não precisa de `s=`/`v=`.
#[must_use]
pub fn png_size(data: &[u8]) -> (u32, u32) {
    if data.len() < 24 || !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return (0, 0);
    }
    let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    (w, h)
}

/// base64 com padding opcional (o app pode mandar sem): `None` = torto.
fn base64_decode(texto: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let limpo: String = texto.chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(limpo.trim_end_matches('='))
        .ok()
}

/// `o=z`: zlib (não gzip, não deflate cru) — é o que o kitty manda.
fn inflate(dados: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(dados)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

#[cfg(test)]
mod checks_graficos {
    use super::{png_size, Formato, Graphics};
    use base64::Engine as _;

    fn b64(dados: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(dados)
    }

    /// 2x2 RGBA de uma cor só — a mesma imagem dos checks do Python.
    fn rgba2x2() -> String {
        b64(&[9, 8, 7, 255].repeat(4))
    }

    fn celula() -> (u32, u32) {
        (10, 19)
    }

    #[test]
    fn imagem_com_offset_e_tamanho_deduzido_da_celula() {
        // `graphics_image_and_offset` do Python: 2x2 numa célula 10x19 = 1x1
        let mut g = Graphics::new();
        let corpo = format!("Ga=T,f=32,s=2,v=2,i=3;{}", rgba2x2());
        let out = g.handle(corpo.as_bytes(), (0, 1), celula());
        assert_eq!(out.respostas, [b"Gi=3;OK".to_vec()]);
        assert!(out.mover);
        assert_eq!(out.linhas, 1);
        let img = g.images().get(&3).expect("imagem guardada");
        assert_eq!((img.width, img.height, img.formato), (2, 2, Formato::Rgba));
        let p = g.placements()[0];
        assert_eq!(
            (p.image_id, p.col, p.linha, p.cols, p.rows),
            (3, 0, 1, 1, 1)
        );
    }

    #[test]
    fn zlib_com_offset_em_pixel_e_c_1_nao_move() {
        // `o=z` + `X=3` + `C=1`: 4x2 RGB numa célula 10x19 = 1x1
        let dados: Vec<u8> = (0..24).collect();
        let mut comp = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut comp, &dados).expect("comprime");
        let zip = comp.finish().expect("zlib");
        let mut g = Graphics::new();
        let corpo = format!("Ga=T,f=24,o=z,s=4,v=2,C=1,X=3;{}", b64(&zip));
        let out = g.handle(corpo.as_bytes(), (0, 0), celula());
        assert!(!out.mover, "C=1 pede para não mexer no cursor");
        let p = g.placements()[0];
        assert_eq!((p.dx, p.cols, p.rows), (3, 1, 1));
    }

    #[test]
    fn transmissao_em_pedacos_so_termina_no_ultimo() {
        let mut g = Graphics::new();
        let tudo = rgba2x2();
        let (a, b) = tudo.split_at(8);
        let primeiro = format!("Ga=t,f=32,s=2,v=2,i=5,m=1;{a}");
        let out = g.handle(primeiro.as_bytes(), (0, 0), celula());
        assert_eq!(out.respostas.len(), 0, "pedaço não responde");
        assert!(g.images().is_empty(), "imagem só existe no fim");
        // o segundo pedaço traz só `m=0`: o control data do primeiro vale
        let segundo = format!("Gm=0;{b}");
        let out = g.handle(segundo.as_bytes(), (0, 0), celula());
        assert_eq!(out.respostas, [b"Gi=5;OK".to_vec()]);
        assert_eq!(g.images()[&5].data, [9, 8, 7, 255].repeat(4));
    }

    #[test]
    fn query_de_capacidade_responde_pelo_meio_e_pelo_formato() {
        // `a=q` do `kitten icat`: direto (d) com 24/32/100 é OK; arquivo não
        let mut g = Graphics::new();
        for (f, ok) in [(24, true), (32, true), (100, true), (16, false)] {
            let corpo = format!("Ga=q,i=1,f={f},t=d");
            let out = g.handle(corpo.as_bytes(), (0, 0), celula());
            let esperado = if ok {
                "Gi=1;OK"
            } else {
                "Gi=1;EINVAL:unsupported transmission"
            };
            assert_eq!(out.respostas, [esperado.as_bytes().to_vec()], "f={f}");
        }
        let out = g.handle(b"Ga=q,i=1,f=32,t=f", (0, 0), celula());
        assert_eq!(
            out.respostas,
            [b"Gi=1;EINVAL:unsupported transmission".to_vec()]
        );
    }

    #[test]
    fn exibir_ja_transmitida_e_enoent_quando_nao_existe() {
        let mut g = Graphics::new();
        g.handle(
            format!("Ga=t,f=32,s=2,v=2,i=9;{}", rgba2x2()).as_bytes(),
            (0, 0),
            celula(),
        );
        let out = g.handle(b"Ga=p,i=9,c=3,r=2", (2, 1), celula());
        assert_eq!(out.respostas, [b"Gi=9;OK".to_vec()]);
        let p = g.placements()[0];
        assert_eq!((p.col, p.linha, p.cols, p.rows), (2, 1, 3, 2));
        let out = g.handle(b"Ga=p,i=404", (0, 0), celula());
        assert_eq!(out.respostas, [b"Gi=404;ENOENT".to_vec()]);
    }

    #[test]
    fn apagar_por_id_e_apagar_tudo() {
        let mut g = Graphics::new();
        for i in [1, 2] {
            g.handle(
                format!("Ga=T,f=32,s=2,v=2,i={i};{}", rgba2x2()).as_bytes(),
                (0, 0),
                celula(),
            );
        }
        assert_eq!(g.placements().len(), 2);
        // `d=i`: uma imagem e a colocação dela
        g.handle(b"Ga=d,d=i,i=1", (0, 0), celula());
        assert!(!g.images().contains_key(&1));
        assert!(g.images().contains_key(&2));
        assert_eq!(g.placements().len(), 1);
        // `d=a`: some tudo
        g.handle(b"Ga=d,d=a", (0, 0), celula());
        assert_eq!(g.images().len(), 0);
        assert_eq!(g.placements(), []);
    }

    #[test]
    fn q_2_silencia_a_resposta() {
        let mut g = Graphics::new();
        let out = g.handle(
            format!("Ga=T,f=32,s=2,v=2,i=4,q=2;{}", rgba2x2()).as_bytes(),
            (0, 0),
            celula(),
        );
        assert_eq!(out.respostas.len(), 0, "q=2 pede silêncio");
        assert_eq!(g.images()[&4].width, 2);
    }

    #[test]
    fn png_deduz_tamanho_do_ihdr() {
        // cabeçalho mínimo: magic + IHDR com 7x5
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&7u32.to_be_bytes());
        png.extend_from_slice(&5u32.to_be_bytes());
        assert_eq!(png_size(&png), (7, 5));
        let mut g = Graphics::new();
        // magic decide o formato mesmo com `f=32` mentindo
        let out = g.handle(
            format!("Ga=t,f=32,i=8;{}", b64(&png)).as_bytes(),
            (0, 0),
            celula(),
        );
        assert_eq!(out.respostas, [b"Gi=8;OK".to_vec()]);
        let img = &g.images()[&8];
        assert_eq!((img.width, img.height, img.formato), (7, 5, Formato::Png));
    }

    #[test]
    fn id_automatico_quando_o_app_nao_manda() {
        let mut g = Graphics::new();
        let out = g.handle(
            format!("Ga=t,f=32,s=2,v=2;{}", rgba2x2()).as_bytes(),
            (0, 0),
            celula(),
        );
        assert_eq!(out.respostas, [b"Gi=1;OK".to_vec()]);
        let out = g.handle(
            format!("Ga=t,f=32,s=2,v=2;{}", rgba2x2()).as_bytes(),
            (0, 0),
            celula(),
        );
        assert_eq!(out.respostas, [b"Gi=2;OK".to_vec()]);
    }

    #[test]
    fn payload_torto_responde_einval_e_nao_guarda_nada() {
        let mut g = Graphics::new();
        let out = g.handle(b"Ga=t,f=32,s=2,v=2,i=1;@@@@", (0, 0), celula());
        assert_eq!(out.respostas, [b"Gi=1;EINVAL:base64".to_vec()]);
        let out = g.handle(b"Ga=t,f=24,o=z,s=2,v=2,i=1;AAAA", (0, 0), celula());
        assert_eq!(out.respostas, [b"Gi=1;EINVAL:zlib".to_vec()]);
        // RGB cru anunciando 2x2 mas com 2 bytes: tamanho mentiu
        let out = g.handle(
            format!("Ga=t,f=24,s=2,v=2,i=1;{}", b64(&[1, 2])).as_bytes(),
            (0, 0),
            celula(),
        );
        assert_eq!(out.respostas, [b"Gi=1;EINVAL:size".to_vec()]);
        assert!(g.images().is_empty(), "nada de imagem pela metade");
    }

    #[test]
    fn rolagem_de_tela_cheia_leva_para_o_historico_e_a_regiao_descarta() {
        let mut g = Graphics::new();
        g.handle(
            format!("Ga=T,f=32,s=2,v=2,i=1;{}", rgba2x2()).as_bytes(),
            (0, 0),
            celula(),
        );
        assert_eq!(g.placements()[0].linha, 0);
        // tela cheia com histórico: a colocação vai para a linha -1
        g.scroll(0, 3, Some(-10));
        assert_eq!(g.placements()[0].linha, -1);
        // e cai fora quando o histórico encurta até não sobrar nada
        g.drop_above(-1);
        assert_eq!(g.placements().len(), 1);
        g.drop_above(0);
        assert_eq!(g.placements(), []);
        // rolagem de região (sem histórico) descarta o que saiu pelo topo
        let mut g = Graphics::new();
        g.handle(
            format!("Ga=T,f=32,s=2,v=2,i=1;{}", rgba2x2()).as_bytes(),
            (0, 0),
            celula(),
        );
        g.scroll(0, 1, None);
        assert_eq!(g.placements(), []);
        // fora da região não se mexe
        let mut g = Graphics::new();
        g.handle(
            format!("Ga=T,f=32,s=2,v=2,i=1;{}", rgba2x2()).as_bytes(),
            (0, 3),
            celula(),
        );
        g.scroll(0, 1, None);
        assert_eq!(g.placements()[0].linha, 3);
    }

    #[test]
    fn animacao_e_controle_sao_ignorados_de_proposito() {
        // H18: `a=f`/`a=a`/`a=c` não respondem nada e não guardam nada
        let mut g = Graphics::new();
        for acao in ["f", "a", "c"] {
            let out = g.handle(
                format!("Ga={acao},i=1;{}", rgba2x2()).as_bytes(),
                (0, 0),
                celula(),
            );
            assert_eq!(out.respostas.len(), 0, "a={acao}");
        }
        assert_eq!(g.images().len(), 0);
    }
}
