//! Learning from your fixes with the learn key: temp folders stand in for your
//! config and run folders.

use std::collections::HashMap;
use std::path::PathBuf;

use localflow_core::{Config, Localflow};
use localflow_desktop::learn::{last_dictation_at, learn_from, learned_message, remember_at};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("localflow-learn-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn localflow(config: PathBuf, vars: &[(&str, &str)]) -> Localflow {
    let vars: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    Localflow::new(Config::from_vars(&|k| vars.get(k).cloned(), config))
}

#[test]
fn remembers_the_last_dictation() {
    let path = temp("last").join("last.json");
    assert_eq!(last_dictation_at(&path), None);
    remember_at(&path, "Push it to GitHub.", 1.5).unwrap();
    assert_eq!(last_dictation_at(&path), Some(("Push it to GitHub.".to_string(), 1.5)));
}

#[test]
fn learn_key() {
    let config = temp("key");
    let lf = localflow(config.clone(), &[]);
    let last = "Push it to git hub and ask Claw.";
    assert_eq!(learn_from(&lf, last, vec!["".into(), "Something else entirely here".into()]), "Select the text you fixed");
    assert_eq!(learn_from(&lf, last, vec![last.into()]), "No changed words found");
    assert_eq!(learn_from(&lf, last, vec!["  ".into(), "Push it to GitHub and ask Claw.".into()]), "Learned: git hub → GitHub");
    assert_eq!(learn_from(&lf, last, vec!["Push it to GitHub and ask Claw.".into()]), "Already learned");
    assert!(std::fs::read_to_string(config.join("words.txt")).unwrap().contains("GitHub: often heard as \"git hub\""));
}

#[test]
fn messages() {
    let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
    assert_eq!(learned_message(&[pair("Claw", "Claude")]), "Learned: Claw → Claude");
    assert_eq!(learned_message(&[pair("Claw", "Claude"), pair("x", "y")]), "Learned 2 words");
    assert_eq!(learned_message(&[pair("a very long misheard phrase", "Something")]), "Learned 1 word");
}
