//! Recherche du n-ième nombre premier (crible segmenté parallèle).
//!
//! Usage : nth_prime <n> <config>
//!   config = 0      : tous les cœurs et tous les threads disponibles
//!   config = XY     : X cœurs, Y threads par cœur (ex. 11, 12, 21, 22, 42...)
//!
//! Sortie : "<premier>, <durée>s, <configuration>"

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use core_affinity::CoreId;

/// Taille d'un segment de crible (1 Mio de booléens, tient dans le cache L2).
const SEGMENT: u64 = 1 << 20;

// ---------------------------------------------------------------------------
// Topologie CPU
// ---------------------------------------------------------------------------

/// Parse une liste de CPU Linux du type "0,4" ou "0-1,8-9".
fn parse_liste_cpu(s: &str) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    for part in s.trim().split(',') {
        if let Some((a, b)) = part.split_once('-') {
            if let (Ok(a), Ok(b)) = (a.trim().parse::<usize>(), b.trim().parse::<usize>()) {
                out.extend(a..=b);
            }
        } else if let Ok(v) = part.trim().parse::<usize>() {
            out.insert(v);
        }
    }
    out
}

/// Retourne la liste des cœurs physiques, chacun étant la liste de ses CPU logiques.
/// S'appuie sur sysfs (Linux) ; en cas d'échec, chaque CPU logique est un cœur.
fn detecter_topologie() -> Vec<Vec<usize>> {
    let ids: BTreeSet<usize> = core_affinity::get_core_ids()
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.id)
        .collect();
    let ids: BTreeSet<usize> = if ids.is_empty() {
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
        (0..n).collect()
    } else {
        ids
    };

    let mut vus: BTreeSet<usize> = BTreeSet::new();
    let mut coeurs: Vec<Vec<usize>> = Vec::new();
    for &id in &ids {
        if vus.contains(&id) {
            continue;
        }
        let chemin = format!("/sys/devices/system/cpu/cpu{id}/topology/thread_siblings_list");
        let mut groupe: Vec<usize> = match fs::read_to_string(chemin) {
            Ok(s) => parse_liste_cpu(&s).into_iter().filter(|c| ids.contains(c)).collect(),
            Err(_) => vec![id],
        };
        if groupe.is_empty() {
            groupe.push(id);
        }
        vus.extend(groupe.iter().copied());
        coeurs.push(groupe);
    }
    coeurs
}

fn pluriel(n: usize, mot: &str) -> String {
    if n > 1 {
        format!("{n} {mot}s")
    } else {
        format!("{n} {mot}")
    }
}

/// Sélectionne les CPU logiques à utiliser et décrit la configuration.
fn choisir_cpus(topo: &[Vec<usize>], config: u32) -> Result<(Vec<usize>, String), String> {
    let total_coeurs = topo.len();
    let total_threads: usize = topo.iter().map(|c| c.len()).sum();
    let max_tpc = topo.iter().map(|c| c.len()).max().unwrap_or(1);

    if config == 0 {
        let cpus = topo.iter().flatten().copied().collect();
        let desc = format!(
            "{} / {} (maximum)",
            pluriel(total_coeurs, "cœur"),
            pluriel(total_threads, "thread")
        );
        return Ok((cpus, desc));
    }

    let x = (config / 10) as usize; // cœurs
    let y = (config % 10) as usize; // threads par cœur
    if x == 0 || y == 0 {
        return Err(format!(
            "Configuration {config} invalide : format attendu XY (X ≥ 1 cœurs, Y ≥ 1 threads par cœur), ou 0 pour le maximum."
        ));
    }

    let eligibles: Vec<&Vec<usize>> = topo.iter().filter(|c| c.len() >= y).collect();
    if eligibles.len() < x {
        return Err(format!(
            "Configuration {config} impossible : {} / {} demandés, \
             alors que cette machine a {} / {} ({} par cœur au maximum).",
            pluriel(x, "cœur"),
            pluriel(x * y, "thread"),
            pluriel(total_coeurs, "cœur"),
            pluriel(total_threads, "thread"),
            pluriel(max_tpc, "thread"),
        ));
    }

    let cpus: Vec<usize> = eligibles
        .into_iter()
        .take(x)
        .flat_map(|c| c.iter().take(y).copied())
        .collect();
    let desc = format!("{} / {}", pluriel(x, "cœur"), pluriel(x * y, "thread"));
    Ok((cpus, desc))
}

// ---------------------------------------------------------------------------
// Crible
// ---------------------------------------------------------------------------

/// Majorant de p_n (Rosser) : p_n < n (ln n + ln ln n) pour n ≥ 6.
fn borne_sup(n: u64) -> u64 {
    if n < 6 {
        15
    } else {
        let x = n as f64;
        (x * (x.ln() + x.ln().ln())).ceil() as u64 + 1
    }
}

/// Crible simple de tous les premiers ≤ max.
fn petits_premiers(max: u64) -> Vec<u64> {
    let n = max as usize;
    let mut crible = vec![true; n + 1];
    crible[0] = false;
    if n >= 1 {
        crible[1] = false;
    }
    let mut i = 2;
    while i * i <= n {
        if crible[i] {
            let mut j = i * i;
            while j <= n {
                crible[j] = false;
                j += i;
            }
        }
        i += 1;
    }
    crible
        .iter()
        .enumerate()
        .filter(|&(_, &p)| p)
        .map(|(i, _)| i as u64)
        .collect()
}

/// Crible le segment [lo, hi) ; `tampon[i]` vaut true si lo+i est premier.
fn cribler(lo: u64, hi: u64, base: &[u64], tampon: &mut Vec<bool>) {
    tampon.clear();
    tampon.resize((hi - lo) as usize, true);
    for &p in base {
        if p * p >= hi {
            break;
        }
        let mut m = ((lo + p - 1) / p * p).max(p * p);
        while m < hi {
            tampon[(m - lo) as usize] = false;
            m += p;
        }
    }
    for v in lo..hi.min(2) {
        tampon[(v - lo) as usize] = false; // 0 et 1
    }
}

/// Calcule le n-ième nombre premier en répartissant les segments sur `cpus`.
fn nieme_premier(n: u64, cpus: &[usize]) -> u64 {
    let limite = borne_sup(n);
    let base = petits_premiers((limite as f64).sqrt() as u64 + 1);
    let nb_seg = (limite / SEGMENT + 1) as usize;
    let compteurs: Vec<AtomicU64> = (0..nb_seg).map(|_| AtomicU64::new(0)).collect();
    let suivant = AtomicUsize::new(0);

    let (base_ref, compteurs_ref, suivant_ref) = (&base, &compteurs, &suivant);
    std::thread::scope(|s| {
        for &cpu in cpus {
            s.spawn(move || {
                core_affinity::set_for_current(CoreId { id: cpu });
                let mut tampon = Vec::new();
                loop {
                    let i = suivant_ref.fetch_add(1, Ordering::Relaxed);
                    if i >= nb_seg {
                        break;
                    }
                    let lo = i as u64 * SEGMENT;
                    let hi = ((i as u64 + 1) * SEGMENT).min(limite + 1);
                    cribler(lo, hi, base_ref, &mut tampon);
                    let c = tampon.iter().filter(|&&b| b).count() as u64;
                    compteurs_ref[i].store(c, Ordering::Relaxed);
                }
            });
        }
    });

    // Localise le segment contenant le n-ième premier, puis le retrouve précisément.
    let mut restant = n;
    let mut tampon = Vec::new();
    for (i, c) in compteurs.iter().enumerate() {
        let c = c.load(Ordering::Relaxed);
        if restant <= c {
            let lo = i as u64 * SEGMENT;
            let hi = ((i as u64 + 1) * SEGMENT).min(limite + 1);
            cribler(lo, hi, &base, &mut tampon);
            let mut vus = 0;
            for (k, &est_premier) in tampon.iter().enumerate() {
                if est_premier {
                    vus += 1;
                    if vus == restant {
                        return lo + k as u64;
                    }
                }
            }
        }
        restant -= c.min(restant);
    }
    unreachable!("la borne supérieure garantit l'existence du n-ième premier");
}

// ---------------------------------------------------------------------------
// Programme principal
// ---------------------------------------------------------------------------

fn executer() -> Result<String, String> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        return Err(format!(
            "Usage : {} <n> <config>\n  config : 0 = maximum, sinon XY (X cœurs, Y threads par cœur), ex. 11, 12, 21, 22",
            args.first().map(String::as_str).unwrap_or("nth_prime")
        ));
    }
    let n: u64 = args[1]
        .replace('_', "")
        .parse()
        .map_err(|_| format!("'{}' n'est pas un entier valide pour n.", args[1]))?;
    if n == 0 {
        return Err("n doit être ≥ 1.".into());
    }
    let config: u32 = args[2]
        .parse()
        .map_err(|_| format!("'{}' n'est pas une configuration valide.", args[2]))?;

    let topo = detecter_topologie();
    let (cpus, desc) = choisir_cpus(&topo, config)?;

    let debut = Instant::now();
    let p = nieme_premier(n, &cpus);
    let duree = debut.elapsed().as_secs_f64();

    Ok(format!("{p}, {duree:.3}s, {desc}"))
}

fn main() -> ExitCode {
    match executer() {
        Ok(s) => {
            println!("{s}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Erreur : {e}");
            ExitCode::from(2)
        }
    }
}
