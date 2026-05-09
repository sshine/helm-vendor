//! `completion` subcommand: emit a shell completion script via `clap_complete`,
//! patched to dynamically complete chart names by re-invoking the binary with
//! the hidden `--chart-names` flag.
//!
//! `clap_complete` only knows how to build static scripts — it has no stable
//! way to wire a positional argument up to a runtime callback. We work around
//! this by post-processing the generator's output per shell:
//!
//! - **zsh**: rewrite the `:_default` action on `::name --` positionals to
//!   call `_helm-vendor_chart_names`, which queries the binary at TAB-time.
//! - **bash**: strip the `[NAME]` / `[VERSION]` metavar literals that leak
//!   into `opts="…"`, rename the generated function to `_clap_inner`, and
//!   wrap it with a thin dispatcher that calls the chart-name helper when
//!   the cursor sits on a chart-name positional.
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
/// `'::name -- <description>:_default'`. Swap the trailing action for our
/// chart-name describer; leave other `:_default` actions (e.g. on `::version`)
/// untouched.
///
/// Also drop the entry advertising the internal `--chart-names` flag.
/// `hide = true` suppresses it in `--help` output but doesn't always
/// propagate through `clap_complete`'s generators.
fn patch_zsh(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + ZSH_HELPER.len());
    for line in raw.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.contains("--chart-names[") {
            continue;
        }
        if trimmed.starts_with("'::name --") && line.contains(":_default'") {
            out.push_str(&line.replacen(":_default'", ":_helm-vendor_chart_names'", 1));
        } else {
            out.push_str(line);
        }
    }
    out.push_str(ZSH_HELPER);
    out
}

/// clap_complete's bash backend bakes positional metavars (`[NAME]`,
/// `[VERSION]`) into the literal `opts="…"` candidate list and offers no
/// hook for runtime values. We strip those metavars, rename the generated
/// function out of the way, and append a wrapper that delegates to it
/// except when the cursor is on a chart-name positional, in which case it
/// produces candidates from `charts.toml`.
fn patch_bash(raw: &str) -> String {
    let renamed = raw.replacen("_helm-vendor() {", "_helm-vendor_clap_inner() {", 1);

    let mut stripped = String::with_capacity(renamed.len());
    for line in renamed.split_inclusive('\n') {
        if line.trim_start().starts_with("opts=\"") {
            let l = line
                .replace(" [NAME]", "")
                .replace(" [VERSION]", "")
                .replace(" --chart-names", "");
            stripped.push_str(&l);
        } else {
            stripped.push_str(line);
        }
    }

    if !stripped.ends_with('\n') {
        stripped.push('\n');
    }
    stripped.push_str(BASH_WRAPPER);
    stripped
}

/// fish doesn't bake metavars into completions, so we just drop the line
/// advertising the internal `--chart-names` flag and append: (1) a
/// chart-name producer and (2) `complete` lines that bind it to chart-name
/// positionals on the relevant subcommands.
fn patch_fish(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + FISH_APPENDIX.len());
    for line in raw.split_inclusive('\n') {
        if line.contains("-l chart-names") {
            continue;
        }
        out.push_str(line);
    }
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

# Wrapper that intercepts chart-name positional completion before delegating
# to the clap_complete-generated function. Detects the active subcommand and
# how many positionals precede the cursor; offers chart names from charts.toml
# only at the chart-name slot of fetch/vendor/check/outdated/update.
_helm-vendor() {
    local cur="${COMP_WORDS[COMP_CWORD]}"
    local subcommand="" count_after=0
    local i
    for ((i=1; i<COMP_CWORD; i++)); do
        local w="${COMP_WORDS[i]}"
        if [[ "$w" == "--config" ]]; then
            ((i++))
            continue
        fi
        if [[ "$w" == -* ]]; then
            continue
        fi
        if [[ -z "$subcommand" ]]; then
            subcommand="$w"
            count_after=0
        else
            count_after=$((count_after + 1))
        fi
    done

    if [[ "$cur" != -* && -n "$subcommand" ]]; then
        case "$subcommand" in
            fetch|vendor|check|outdated|update)
                if (( count_after == 0 )); then
                    COMPREPLY=( $(compgen -W "$(_helm-vendor_chart_names_helper)" -- "$cur") )
                    return 0
                fi
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

complete -c helm-vendor -n "__fish_helm_vendor_using_subcommand fetch"    -f -a "(__helm_vendor_chart_names)" -d 'chart'
complete -c helm-vendor -n "__fish_helm_vendor_using_subcommand check"    -f -a "(__helm_vendor_chart_names)" -d 'chart'
complete -c helm-vendor -n "__fish_helm_vendor_using_subcommand outdated" -f -a "(__helm_vendor_chart_names)" -d 'chart'
complete -c helm-vendor -n "__fish_helm_vendor_using_subcommand update; and not __fish_seen_subcommand_from (__helm_vendor_chart_names)" -f -a "(__helm_vendor_chart_names)" -d 'chart'
"#;
