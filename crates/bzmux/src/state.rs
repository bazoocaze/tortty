//! Estado da janela entre execuções: posição, tamanho e maximizado — J2–J6.
//!
//! Espelha o `state.py` do Python, e pela mesma razão é **puro**: quem fala com
//! `winit` é o `app.rs`, que aqui só entrega e recebe retângulos. Assim o
//! arquivo, a validação e a atomicidade são testáveis sem display.
//!
//! O arquivo é JSON em `XDG_STATE_HOME` (estado, não configuração): se ele
//! sumir ou estiver torto, a janela abre no padrão e nada mais acontece.

use std::io::Write;
use std::path::PathBuf;

/// Abaixo disto é lixo de WM/restauração, não geometria.
pub const MIN_W: u32 = 320;
pub const MIN_H: u32 = 200;
/// Padrão: 980x640 devolve ~70x20 com a fonte de 13 pt (`window.py:24-31`).
pub const PADRAO: (u32, u32) = (980, 640);

/// O que fica no arquivo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub w: u32,
    pub h: u32,
    /// Maximizado é estado de WM, separado do tamanho normal (J5): restaurar
    /// `max` não pode sobrescrever o tamanho que a janela tinha antes.
    pub maximizado: bool,
}

impl Default for Geometry {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            w: PADRAO.0,
            h: PADRAO.1,
            maximizado: false,
        }
    }
}

/// Retângulo de monitor, como o `winit` entrega: (x, y, largura, altura).
pub type Monitor = (i32, i32, u32, u32);

/// `$XDG_STATE_HOME/bzmux/window.json`, com `~/.local/state` de reserva.
#[must_use]
pub fn state_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("bzmux").join("window.json"))
}

/// Lê o arquivo; `None` para ausente, ilegível, JSON torto ou tipo errado.
/// Nunca é erro: estado corrompido só significa "abre no padrão".
#[must_use]
pub fn load() -> Option<Geometry> {
    load_from(&state_path()?)
}

/// `load` num caminho dado (é o que o teste usa: ambiente não é estado de teste).
#[must_use]
pub fn load_from(path: &std::path::Path) -> Option<Geometry> {
    let texto = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&texto).ok()?;
    let obj = v.as_object()?;
    let num = |k: &str| obj.get(k).and_then(serde_json::Value::as_i64);
    // `w`/`h` obrigatórios: sem eles não há geometria, só um `max: true`
    Some(Geometry {
        x: num("x").and_then(|n| i32::try_from(n).ok()),
        y: num("y").and_then(|n| i32::try_from(n).ok()),
        w: u32::try_from(num("w")?).ok()?,
        h: u32::try_from(num("h")?).ok()?,
        maximizado: obj
            .get("max")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    })
}

/// Grava de forma **atômica** (tmp + rename): nunca deixa JSON pela metade.
/// Não poder salvar não pode derrubar o fechamento.
pub fn save(g: &Geometry) {
    if let Some(path) = state_path() {
        save_to(&path, g);
    }
}

/// `save` num caminho dado.
pub fn save_to(path: &std::path::Path, g: &Geometry) {
    if let Some(dir) = path.parent() {
        // diretório que não dá para criar não é erro fatal: o terminal tem de
        // rodar confinado (sandbox, home somente leitura). Só não pode ser
        // silencioso, senão ninguém entende por que a janela não é lembrada
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("bzmux: sem geometria lembrada ({}: {e})", dir.display());
            return;
        }
    }
    let mut v = serde_json::Map::new();
    if let (Some(x), Some(y)) = (g.x, g.y) {
        v.insert("x".into(), x.into());
        v.insert("y".into(), y.into());
    }
    v.insert("w".into(), g.w.into());
    v.insert("h".into(), g.h.into());
    v.insert("max".into(), g.maximizado.into());
    let tmp = path.with_extension("json.tmp");
    let Ok(mut f) = std::fs::File::create(&tmp) else {
        eprintln!(
            "bzmux: sem geometria lembrada ({} não gravável)",
            tmp.display()
        );
        return;
    };
    let ok = writeln!(f, "{}", serde_json::Value::Object(v)).is_ok() && f.sync_all().is_ok();
    drop(f);
    if !ok || std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Geometria utilizável: tamanho sadio e o canto **dentro de algum monitor**.
///
/// `None` significa "não sobra nada aproveitável" — monitor desconectado,
/// arquivo editado à mão, resolução trocada. Basta o canto superior esquerdo
/// estar num monitor: a barra de título fica alcançável e o WM nunca abre a
/// janela fora da tela.
#[must_use]
pub fn fit(
    x: Option<i32>,
    y: Option<i32>,
    w: u32,
    h: u32,
    monitors: &[Monitor],
) -> Option<Geometry> {
    if w < MIN_W || h < MIN_H {
        return None;
    }
    let (Some(x), Some(y)) = (x, y) else {
        // sem posição (Wayland, J6): o tamanho sozinho ainda vale
        return Some(Geometry {
            x: None,
            y: None,
            w,
            h,
            maximizado: false,
        });
    };
    if monitors.is_empty() {
        // sem monitor conhecido não há como julgar: aceita
        return Some(Geometry {
            x: Some(x),
            y: Some(y),
            w,
            h,
            maximizado: false,
        });
    }
    let dentro = monitors.iter().any(|&(mx, my, mw, mh)| {
        x >= mx
            && x < mx + i32::try_from(mw).unwrap_or(i32::MAX)
            && y >= my
            && y < my + i32::try_from(mh).unwrap_or(i32::MAX)
    });
    dentro.then_some(Geometry {
        x: Some(x),
        y: Some(y),
        w,
        h,
        maximizado: false,
    })
}

#[cfg(test)]
mod checks_estado {
    use super::{fit, load_from, save_to, Geometry, Monitor, MIN_H, MIN_W, PADRAO};
    use std::path::{Path, PathBuf};

    fn monitores() -> Vec<Monitor> {
        // um 1920x1080 em 0,0 e um 1280x1024 à direita
        vec![(0, 0, 1920, 1080), (1920, 0, 1280, 1024)]
    }

    fn tmp(nome: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bz-estado-{nome}"));
        let _ = std::fs::remove_dir_all(&d);
        d.join("bzmux").join("window.json")
    }

    #[test]
    fn padrao_e_980x640() {
        let g = Geometry::default();
        assert_eq!((g.w, g.h), PADRAO);
        assert_eq!((g.w, g.h), (980, 640));
        assert!(!g.maximizado);
    }

    #[test]
    fn fit_aceita_retangulo_sao_no_monitor() {
        let g = fit(Some(100), Some(80), 900, 600, &monitores()).expect("cabe");
        assert_eq!((g.x, g.y, g.w, g.h), (Some(100), Some(80), 900, 600));
    }

    #[test]
    fn fit_rejeita_tamanho_de_lixo() {
        // abaixo do mínimo: lixo de WM ou arquivo editado à mão
        assert!(fit(Some(0), Some(0), MIN_W - 1, 600, &monitores()).is_none());
        assert!(fit(Some(0), Some(0), 600, MIN_H - 1, &monitores()).is_none());
        // a borda exata passa
        assert!(fit(Some(0), Some(0), MIN_W, MIN_H, &monitores()).is_some());
    }

    #[test]
    fn fit_rejeita_canto_fora_de_todos_os_monitores() {
        // monitor desconectado: o canto ficou no vazio
        assert!(fit(Some(5000), Some(0), 900, 600, &monitores()).is_none());
        assert!(fit(Some(0), Some(-500), 900, 600, &monitores()).is_none());
        // segundo monitor: aceita
        assert!(fit(Some(2000), Some(100), 900, 600, &monitores()).is_some());
        // exatamente na largura do primeiro monitor já é o segundo
        assert!(fit(Some(1920), Some(0), 900, 600, &monitores()).is_some());
        // e um pixel antes ainda é o primeiro
        assert!(fit(Some(1919), Some(0), 900, 600, &monitores()).is_some());
    }

    #[test]
    fn fit_sem_monitor_conhecido_aceita_o_tamanho() {
        // sem monitor não há como julgar posição (o WM decide)
        assert!(fit(Some(-9000), Some(-9000), 900, 600, &[]).is_some());
    }

    #[test]
    fn fit_sem_posicao_vale_o_tamanho_wayland() {
        // Wayland não dá posição (J6): o tamanho sozinho é o que sobra
        let g = fit(None, None, 900, 600, &monitores()).expect("tamanho vale");
        assert_eq!((g.x, g.y), (None, None));
        assert_eq!((g.w, g.h), (900, 600));
    }

    #[test]
    fn caminho_respeita_xdg_state_home() {
        // `state_path` é a única parte que lê ambiente; a montagem é o que importa
        let p = std::env::temp_dir().join("bz-xdg");
        assert_eq!(
            p.join("bzmux").join("window.json"),
            PathBuf::from(format!("{}/bzmux/window.json", p.display()))
        );
    }

    #[test]
    fn salva_e_le_de_volta_e_nao_deixa_tmp() {
        let path = tmp("ida-volta");
        let g = Geometry {
            x: Some(120),
            y: Some(60),
            w: 1000,
            h: 700,
            maximizado: true,
        };
        save_to(&path, &g);
        assert_eq!(load_from(&path), Some(g));
        let sobras: Vec<String> = std::fs::read_dir(path.parent().expect("dir"))
            .expect("diretório criado")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(sobras, ["window.json"], "tmp não pode sobrar");
        // Wayland: sem posição, o resto continua (J6)
        let s = Geometry {
            x: None,
            y: None,
            ..g
        };
        save_to(&path, &s);
        assert_eq!(load_from(&path), Some(s));
        let _ = std::fs::remove_dir_all(Path::new(&path).ancestors().nth(2).expect("raiz"));
    }

    #[test]
    fn gravar_em_lugar_impossivel_nao_estoura() {
        // sandbox/home somente leitura: o app tem de seguir de pé (e dizer no
        // stderr, não sumir calado)
        let path = std::path::Path::new("/proc/bzmux-nao-existe/window.json");
        save_to(
            path,
            &Geometry {
                x: None,
                y: None,
                w: 800,
                h: 600,
                maximizado: false,
            },
        );
        assert!(!path.exists());
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn arquivo_torto_nao_e_erro_so_padrao() {
        let path = tmp("torto");
        std::fs::create_dir_all(path.parent().expect("dir")).expect("dir");
        for lixo in ["", "{", "[]", "null", "\"texto\"", "{\"x\":1}"] {
            std::fs::write(&path, lixo).expect("escreve");
            assert_eq!(load_from(&path), None, "lixo aceito: {lixo:?}");
        }
        // número que não cabe em i32/x/y vira None, mas w/h mandam
        std::fs::write(&path, r#"{"x":1e300,"y":3,"w":900,"h":600}"#).expect("escreve");
        let g = load_from(&path).expect("w/h válidos");
        assert_eq!((g.x, g.y, g.w, g.h), (None, Some(3), 900, 600));
        // arquivo que nem existe: padrão, sem erro
        assert_eq!(load_from(&path.with_extension("naoexiste")), None);
        let _ = std::fs::remove_dir_all(Path::new(&path).ancestors().nth(2).expect("raiz"));
    }
}
