//! Recherche du n-ième nombre premier (crible segmenté parallèle, impairs + bitset).
//!
//! Usage : burn_prems <n> <config>      (n accepte les séparateurs `_` : 10_000_000)
//!   config = 0      : tous les cœurs et tous les threads disponibles
//!   config = XY     : X cœurs, Y threads par cœur (ex. 11, 12, 21, 22, 42...)
//!
//! Sortie : "<premier>, <durée>s, <configuration>"
//!
//! Optimisations (v3) :
//!  - pré-crible : les multiples de 3, 5, 7, 11, 13 sont posés d'un coup en recopiant un motif
//!    périodique (période 15015 bits) au lieu d'être marqués un par un ;
//! Optimisations (v2) :
//!  - seuls les nombres impairs sont représentés (l'indice k ↔ le nombre 2k+1) ;
//!  - un bit par nombre (au lieu d'un octet) : 16× moins de mémoire par plage ;
//!  - segments de 64 Kio, qui tiennent dans le cache L2 (et ne polluent pas le L1 des voisins).

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

use core_affinity::CoreId;

/// Petits premiers impairs traités par pré-crible (motif périodique).
const PRE: [u64; 5] = [3, 5, 7, 11, 13];

const fn produit(l: &[u64]) -> usize {
    let mut i = 0;
    let mut r = 1usize;
    while i < l.len() {
        r *= l[i] as usize;
        i += 1;
    }
    r
}

/// Période du motif, en indices d'impairs : 3·5·7·11·13 = 15015.
const PERIODE: usize = produit(&PRE);

/// Taille de segment par défaut : 2^19 impairs = 2^19 bits = 64 Kio (2^20 nombres couverts).
/// Modifiable à l'exécution avec la variable d'environnement SEG_LOG2 (exposant, 14..=26).
const SEG_LOG2_DEFAUT: u32 = 19;

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
// Crible (impairs, bitset)
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

/// Tous les premiers impairs ≤ max (crible simple).
fn petits_premiers_impairs(max: usize) -> Vec<u64> {
    let mut crible = vec![true; max + 1];
    let mut i = 2;
    while i * i <= max {
        if crible[i] {
            let mut j = i * i;
            while j <= max {
                crible[j] = false;
                j += i;
            }
        }
        i += 1;
    }
    (3..=max).filter(|&i| crible[i]).map(|i| i as u64).collect()
}

/// Motif de pré-crible : le bit i vaut 1 si le nombre 2i+1 est multiple d'un premier de PRE.
/// Il est périodique de période PERIODE ; on le déroule sur PERIODE + un segment (+ marge)
/// pour pouvoir le recopier à n'importe quel décalage.
fn construire_motif(seg_odds: u64) -> Vec<u64> {
    let nbits = PERIODE + seg_odds as usize + 256;
    let mut m = vec![0u64; nbits / 64 + 2];
    let total = m.len() * 64;
    for &p in PRE.iter() {
        let mut k = ((p - 1) / 2) as usize;
        while k < total {
            m[k >> 6] |= 1u64 << (k & 63);
            k += p as usize;
        }
    }
    m
}

/// Crible les impairs d'indices k ∈ [klo, khi) (le nombre est 2k+1).
/// Après l'appel, un bit à 0 = premier, un bit à 1 = composé (ou bourrage de fin de segment).
fn cribler(klo: u64, khi: u64, base: &[u64], motif: &[u64], mots: &mut Vec<u64>) {
    let nbits = (khi - klo) as usize;
    let nmots = (nbits + 63) / 64;
    mots.clear();
    mots.resize(nmots, 0);

    // Pré-crible : recopie du motif décalé de (klo mod PERIODE) bits.
    let s = (klo % PERIODE as u64) as usize;
    let (q, r) = (s >> 6, (s & 63) as u32);
    if r == 0 {
        mots.copy_from_slice(&motif[q..q + nmots]);
    } else {
        let src = &motif[q..q + nmots + 1];
        for (w, paire) in mots.iter_mut().zip(src.windows(2)) {
            *w = (paire[0] >> r) | (paire[1] << (64 - r));
        }
    }
    if klo == 0 {
        // Le motif marque aussi les petits premiers eux-mêmes : on les « démarque ».
        for &p in PRE.iter() {
            let k = ((p - 1) / 2) as usize;
            if k < nbits {
                mots[k >> 6] &= !(1u64 << (k & 63));
            }
        }
    }

    let lo_num = 2 * klo + 1; // plus petit nombre du segment
    let hi_num = 2 * khi; // tous les nombres du segment sont < hi_num

    for &p in base {
        let p2 = p * p;
        if p2 >= hi_num {
            break;
        }
        // Premier multiple impair de p à marquer : ≥ p² et ≥ lo_num.
        let m = if p2 >= lo_num {
            p2
        } else {
            let m = (lo_num + p - 1) / p * p;
            if m % 2 == 0 { m + p } else { m }
        };
        let mut i = ((m - 1) / 2 - klo) as usize;
        let pas = p as usize; // deux impairs consécutifs multiples de p : écart p en indices
        while i < nbits {
            // SAFETY : i < nbits ≤ nmots * 64, donc i >> 6 < nmots.
            unsafe {
                *mots.get_unchecked_mut(i >> 6) |= 1u64 << (i & 63);
            }
            i += pas;
        }
    }

    if klo == 0 {
        mots[0] |= 1; // le nombre 1 n'est pas premier
    }
    let reste = nbits & 63;
    if reste != 0 {
        mots[nmots - 1] |= !0u64 << reste; // bourrage : bits hors segment = « composés »
    }
}

/// Calcule le n-ième nombre premier en répartissant les segments sur `cpus`.
fn nieme_premier(n: u64, cpus: &[usize], seg_odds: u64) -> u64 {
    if n == 1 {
        return 2;
    }
    let limite = borne_sup(n);
    let base: Vec<u64> = petits_premiers_impairs((limite as f64).sqrt() as usize + 1)
        .into_iter()
        .filter(|p| !PRE.contains(p)) // déjà traités par le pré-crible
        .collect();
    let motif = construire_motif(seg_odds);
    let total_k = (limite + 1) / 2; // nombre d'impairs ≤ limite
    let nb_seg = ((total_k + seg_odds - 1) / seg_odds) as usize;
    let compteurs: Vec<AtomicU64> = (0..nb_seg).map(|_| AtomicU64::new(0)).collect();
    let suivant = AtomicUsize::new(0);

    let (base_ref, compteurs_ref, suivant_ref, motif_ref) = (&base, &compteurs, &suivant, &motif);
    std::thread::scope(|s| {
        for &cpu in cpus {
            s.spawn(move || {
                core_affinity::set_for_current(CoreId { id: cpu });
                let mut mots = Vec::new();
                loop {
                    let i = suivant_ref.fetch_add(1, Ordering::Relaxed);
                    if i >= nb_seg {
                        break;
                    }
                    let klo = i as u64 * seg_odds;
                    let khi = (klo + seg_odds).min(total_k);
                    cribler(klo, khi, base_ref, motif_ref, &mut mots);
                    let premiers: u64 = mots.iter().map(|w| w.count_zeros() as u64).sum();
                    compteurs_ref[i].store(premiers, Ordering::Relaxed);
                }
            });
        }
    });

    // Le n-ième premier est le (n-1)-ième premier impair. On repère son segment...
    let mut restant = n - 1;
    let mut mots = Vec::new();
    for (i, c) in compteurs.iter().enumerate() {
        let c = c.load(Ordering::Relaxed);
        if restant > c {
            restant -= c;
            continue;
        }
        // ... puis on le retrouve précisément en recriblant ce segment.
        let klo = i as u64 * seg_odds;
        let khi = (klo + seg_odds).min(total_k);
        cribler(klo, khi, &base, &motif, &mut mots);
        for (j, &w) in mots.iter().enumerate() {
            let mut libres = !w; // bits à 1 = premiers
            let nb = libres.count_ones() as u64;
            if restant > nb {
                restant -= nb;
                continue;
            }
            for _ in 1..restant {
                libres &= libres - 1; // efface le bit de poids faible
            }
            let k = klo + (j as u64) * 64 + libres.trailing_zeros() as u64;
            return 2 * k + 1;
        }
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
            args.first().map(String::as_str).unwrap_or("burn_prems")
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

    // Taille de segment réglable pour les essais : SEG_LOG2=21 burn_prems 1_000_000_000 0
    let (seg_log2, perso) = match env::var("SEG_LOG2") {
        Ok(v) => match v.trim().parse::<u32>() {
            Ok(e) if (14..=26).contains(&e) => (e, true),
            _ => return Err(format!("SEG_LOG2='{v}' invalide : entier entre 14 et 26 attendu.")),
        },
        Err(_) => (SEG_LOG2_DEFAUT, false),
    };

    let debut = Instant::now();
    let p = nieme_premier(n, &cpus, 1u64 << seg_log2);
    let duree = debut.elapsed().as_secs_f64();

    if perso {
        Ok(format!("{p}, {duree:.3}s, {desc}, segment 2^{seg_log2} bits"))
    } else {
        Ok(format!("{p}, {duree:.3}s, {desc}"))
    }
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
