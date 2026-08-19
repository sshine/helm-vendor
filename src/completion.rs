//! `completion` subcommand: emit a shell completion script via `clap_complete`,
//! patched to dynamically complete chart names by re-invoking the binary with
//! the hidden `--chart-names` flag.
//!
//! `clap_complete` only knows how to build static scripts — it has no stable
//! way to wire a positional argument up to a runtime callback. We work around
//! this by post-processing the generator's output per shell:
//!
//! - **zsh**: rewrite the `:_default` action on `*::names --` positionals to
//!   call `_helm-vendor_chart_names`, which queries the binary at TAB-time.
//! - **bash**: strip the positional metavars that leak into `opts="…"`, rename
//!   the generated function to `_clap_inner`, and wrap it with a thin
//!   dispatcher that calls the chart-name helper when the cursor sits on a
//!   chart-name positional.
//! - **fish**: append `complete` lines that bind chart-name positionals on
//!   the relevant subcommands to a chart-name producer function.
//!
//! When no `charts.toml` is reachable the chart-name lookup exits 0 with no
//! output, so completion never errors out for users who happen to be elsewhere
//! on disk.

use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::CommandFactory;

use crate::args::{Args, Shell};

pub fn run(config: Option<PathBuf>, shell: Option<Shell>, chart_names: bool) -> ExitCode {
    if chart_names {
        return print_chart_names(config);
    }
    let Some(shell) = shell else {
        // Should be unreachable: clap's `required_unless_present` enforces
        // that one of `shell` or `--chart-names` is supplied.
        eprintln!("error: shell argument required");
        return ExitCode::FAILURE;
    };

    let mut cmd = Args::command();
    let bin = "helm-vendor";
    let mut buf: Vec<u8> = Vec::new();
    let cc_shell: clap_complete::Shell = match shell {
        Shell::Bash => clap_complete::Shell::Bash,
        Shell::Zsh => clap_complete::Shell::Zsh,
        Shell::Fish => clap_complete::Shell::Fish,
    };
    clap_complete::generate(cc_shell, &mut cmd, bin, &mut buf);
    let raw = String::from_utf8(buf).expect("clap_complete emits UTF-8");

    let patched = match shell {
        Shell::Bash => patch_bash(&raw),
        Shell::Zsh => patch_zsh(&raw),
        Shell::Fish => patch_fish(&raw),
    };
    print!("{patched}");
    let _ = io::Write::flush(&mut io::stdout());
    ExitCode::SUCCESS
}

fn print_chart_names(config: Option<PathBuf>) -> ExitCode {
    let Some(path) = config else {
        return ExitCode::SUCCESS;
    };
    let Ok(registry) = crate::chart::load(&path) else {
        return ExitCode::SUCCESS;
    };
    for name in registry.charts.keys() {
        println!("{name}");
    }
    ExitCode::SUCCESS
}

/// In the generated zsh script, every chart-name positional renders as
/// `'*::names -- <description>:_default'`. Swap the trailing action for our
/// chart-name describer; leave other `:_default` actions (notably `add`'s
/// `:name`, which names a chart that does not exist yet) untouched.
fn patch_zsh(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + ZSH_HELPER.len());
    for line in raw.split_inclusive('\n') {
        if line.trim_start().starts_with("'*::names --") && line.contains(":_default'") {
            out.push_str(&line.replacen(":_default'", ":_helm-vendor_chart_names'", 1));
        } else {
            out.push_str(line);
        }
    }
    out.push_str(ZSH_HELPER);
    out
}

/// clap_complete's bash backend bakes positional metavars (`[NAMES]...`,
/// `<URL>`) into the literal `opts="…"` candidate list and offers no hook for
/// runtime values. We strip those metavars, rename the generated function out
/// of the way, and append a wrapper that delegates to it except when the cursor
/// is on a chart-name positional, in which case it produces candidates from
/// `charts.toml`.
fn patch_bash(raw: &str) -> String {
    let renamed = raw.replacen("_helm-vendor() {", "_helm-vendor_clap_inner() {", 1);

    let mut stripped = String::with_capacity(renamed.len());
    for line in renamed.split_inclusive('\n') {
        stripped.push_str(&strip_metavars(line));
    }

    if !stripped.ends_with('\n') {
        stripped.push('\n');
    }
    stripped.push_str(BASH_WRAPPER);
    stripped
}

/// Drop `<FOO>` / `[FOO]...` tokens from an `opts="…"` assignment, leaving the
/// flags. Matching on the bracket rather than on a list of metavar names means
/// renaming an argument cannot silently reintroduce the literal.
fn strip_metavars(line: &str) -> String {
    let Some(start) = line.find("opts=\"") else {
        return line.to_string();
    };
    let open = start + "opts=\"".len();
    let Some(len) = line[open..].find('"') else {
        return line.to_string();
    };
    let kept: Vec<&str> = line[open..open + len]
        .split_whitespace()
        .filter(|token| !token.starts_with('<') && !token.starts_with('['))
        .collect();
    format!("{}{}{}", &line[..open], kept.join(" "), &line[open + len..])
}

/// fish doesn't bake metavars into completions, so we only append: (1) a
/// chart-name producer and (2) `complete` lines that bind it to chart-name
/// positionals on the relevant subcommands.
fn patch_fish(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + FISH_APPENDIX.len());
    out.push_str(raw);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(FISH_APPENDIX);
    out
}

const ZSH_HELPER: &str = r#"
# Dynamic chart-name candidates: re-invokes the binary with the hidden
# `--chart-names` flag, forwarding `--config` if the user typed it on
# the line. Silent on missing/invalid charts.toml.
_helm-vendor_chart_names() {
    local -a config_arg names
    local i
    for ((i=2; i <= $#words; i++)); do
        if [[ "${words[i]}" == "--config" ]]; then
            config_arg=(--config "${words[i+1]}")
            break
        elif [[ "${words[i]}" == "--config="* ]]; then
            config_arg=("${words[i]}")
            break
        fi
    done
    names=(${(f)"$(helm-vendor "${config_arg[@]}" completion --chart-names 2>/dev/null)"})
    _describe 'chart' names
}
"#;

const BASH_WRAPPER: &str = r#"
# Helper: reinvoke the binary to enumerate chart names from charts.toml,
# forwarding --config if it was supplied on the command line. Silent on
# missing/invalid charts.toml.
_helm-vendor_chart_names_helper() {
    local -a config_arg=()
    local i
    for ((i=1; i<COMP_CWORD; i++)); do
        local w="${COMP_WORDS[i]}"
        case "$w" in
            --config)
                config_arg=(--config "${COMP_WORDS[i+1]}")
                ((i++))
                ;;
            --config=*)
                config_arg=("$w")
                ;;
        esac
    done
    helm-vendor "${config_arg[@]}" completion --chart-names 2>/dev/null
}

# Wrapper that intercepts chart-name positional completion before delegating to
# the clap_complete-generated function. The chart-selecting subcommands take a
# variadic list, so every positional slot is a chart-name slot.
_helm-vendor() {
    local cur="${COMP_WORDS[COMP_CWORD]}"
    local prev="${COMP_WORDS[COMP_CWORD-1]}"
    local subcommand="" i

    # An option that takes a value owns the following word.
    case "$prev" in
        --config|--to)
            _helm-vendor_clap_inner "$@"
            return
            ;;
    esac

    for ((i=1; i<COMP_CWORD; i++)); do
        local w="${COMP_WORDS[i]}"
        case "$w" in
            --config|--to)
                ((i++))
                continue
                ;;
            -*)
                continue
                ;;
        esac
        if [[ -z "$subcommand" ]]; then
            subcommand="$w"
        fi
    done

    if [[ "$cur" != -* ]]; then
        case "$subcommand" in
            sync|fetch|vendor|update|list|ls|remove|rm)
                COMPREPLY=( $(compgen -W "$(_helm-vendor_chart_names_helper)" -- "$cur") )
                return 0
                ;;
        esac
    fi

    _helm-vendor_clap_inner "$@"
}
"#;

const FISH_APPENDIX: &str = r#"
function __helm_vendor_chart_names
    set -l config_arg
    set -l tokens (commandline -opc)
    set -l n (count $tokens)
    for i in (seq 2 $n)
        if test "$tokens[$i]" = "--config"
            set -l j (math $i + 1)
            if test $n -ge $j
                set config_arg --config $tokens[$j]
            end
            break
        else
            switch $tokens[$i]
                case '--config=*'
                    set config_arg $tokens[$i]
                    break
            end
        end
    end
    helm-vendor $config_arg completion --chart-names 2>/dev/null
end

complete -c helm-vendor -n "__fish_helm_vendor_using_subcommand sync"   -f -a "(__helm_vendor_chart_names)" -d 'chart'
complete -c helm-vendor -n "__fish_helm_vendor_using_subcommand update" -f -a "(__helm_vendor_chart_names)" -d 'chart'
complete -c helm-vendor -n "__fish_helm_vendor_using_subcommand list"   -f -a "(__helm_vendor_chart_names)" -d 'chart'
complete -c helm-vendor -n "__fish_helm_vendor_using_subcommand remove" -f -a "(__helm_vendor_chart_names)" -d 'chart'
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metavars_are_stripped_but_flags_survive() {
        let line = r#"            opts="-h --to --no-fetch --config --help [NAMES]...""#;
        assert_eq!(
            strip_metavars(line),
            r#"            opts="-h --to --no-fetch --config --help""#
        );
    }

    #[test]
    fn required_metavars_are_stripped_too() {
        let line = r#"opts="-h --no-fetch --config --help <NAME> <URL>""#;
        assert_eq!(
            strip_metavars(line),
            r#"opts="-h --no-fetch --config --help""#
        );
    }

    #[test]
    fn lines_without_opts_are_untouched() {
        let line = "    case \"${cmd}\" in\n";
        assert_eq!(strip_metavars(line), line);
    }

    #[test]
    fn subcommand_candidates_are_not_treated_as_metavars() {
        let line = r#"opts="add remove sync update list completion help""#;
        assert_eq!(strip_metavars(line), line);
    }

    #[test]
    fn zsh_rewrites_only_variadic_chart_name_positionals() {
        let raw = concat!(
            "':name -- Chart name:_default' \\\n",
            "'*::names -- Chart names. Omit for every chart in charts.toml:_default' \\\n",
        );
        let patched = patch_zsh(raw);
        assert!(patched.contains("':name -- Chart name:_default'"));
        assert!(patched.contains("'*::names -- Chart names. Omit for every chart in charts.toml:_helm-vendor_chart_names'"));
    }
}
