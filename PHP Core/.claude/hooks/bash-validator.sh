#!/bin/bash
# Bash Validator Hook
# Blocks destructive commands for native PHP projects.
# Hook type: PreToolUse:Bash
# Exit codes: 0 = pass, 2 = block. The repetition guard at the end warns
# before it blocks; how a warning reaches the agent is described there.

# Consume the complete hook payload without relying on external utilities.
# `read -d ''` returns nonzero at EOF, which is the expected delimiter here.
INPUT=
IFS= read -r -d '' INPUT || :

# Cheap self-filter before any process is forked: Codex and Cursor register
# this hook without a tool matcher, so it runs for every tool call. A real
# "command"/"cmd" JSON key always appears unescaped on the wire, while the
# same text inside a string value arrives as \"command\" and does not match,
# so payloads that cannot carry a shell command exit here for free.
case "$INPUT" in
  *'"command"'*|*'"cmd"'*) ;;
  *) exit 0 ;;
esac

# Decode JSON instead of scraping quoted strings; nested shell commands contain escaped quotes.
extract_command() {
  if command -v jq >/dev/null 2>&1; then
    jq -r '[.. | objects | .command?, .cmd? | select(type == "string" and length > 0)] | join("\n")'
  elif command -v php >/dev/null 2>&1; then
    php -r '$v=json_decode(stream_get_contents(STDIN), true); $found=[]; $find=function($v) use (&$find, &$found) { if (!is_array($v)) return; foreach (["command", "cmd"] as $k) if (isset($v[$k]) && is_string($v[$k]) && $v[$k] !== "") $found[]=$v[$k]; foreach ($v as $child) $find($child); }; $find($v); echo implode("\n", $found);'
  elif command -v python3 >/dev/null 2>&1; then
    python3 -c 'import json,sys
found=[]
def find(v):
    if isinstance(v, dict):
        for key in ("command", "cmd"):
            if isinstance(v.get(key), str) and v[key]: found.append(v[key])
        for child in v.values():
            find(child)
    elif isinstance(v, list):
        for child in v:
            find(child)
find(json.load(sys.stdin))
print("\n".join(found), end="")'
  else
    return 1
  fi
}

if ! command -v jq >/dev/null 2>&1 && ! command -v php >/dev/null 2>&1 && ! command -v python3 >/dev/null 2>&1; then
  echo "bash-validator: no JSON extractor available (jq/php/python3), validation skipped" >&2
  exit 0
fi

COMMAND=$(printf '%s' "$INPUT" | extract_command 2>/dev/null) || exit 0

if [ -z "$COMMAND" ]; then
  exit 0
fi

# >>> bash-validator generic section >>>
# Everything from this marker to the closing one is byte-identical in the
# Laravel, Symfony, PHP Core and WordPress editions and in
# Infrastructure-Creator (tests/test_bash_validator_corpus.py fails on any
# drift): edit it in one copy, then paste the block into the other four.
# Framework rules live below the closing marker, in BV_FRAMEWORK_RULES.
#
# The command is parsed once, in pure bash and without a fork per rule, into
# the simple commands the shell would run:
#   - backslash-newline continuations are joined and quotes are removed;
#   - segments split on ; && || | & ( ) and newlines;
#   - $(...), `...` and <(...) bodies become segments of their own, and so
#     do the string argument of sh/bash -c (also after -euo pipefail), su -c,
#     eval and git submodule foreach, and a heredoc or here-string fed to a
#     shell that reads stdin, directly or through a launcher (docker exec -i
#     app bash, ssh host bash -s), or to ssh HOST with no remote command;
#   - any other heredoc body is data: only the SQL rules of the segment that
#     owns it read it;
#   - VAR=value prefixes, wrappers (command, builtin, exec, nohup, time,
#     sudo, doas, env, nice, ionice, timeout, stdbuf, xargs) and a binary's
#     directory (/usr/bin/git -> git) are stripped; `php script` runs the
#     script, so `php artisan` is the program `artisan`; a launcher with no
#     rules of its own (docker compose exec app, ddev, lando, ssh host) is
#     checked as the first later word that has rules, and a quoted command
#     line given to a launcher in BV_LAUNCHERS (ssh host '...', ddev exec
#     "...", vagrant ssh -c "...") is checked as a command of its own.
# Git and flag rules are case-sensitive (git branch -d is not -D); console
# command names follow Symfony Console (any unambiguous per-part prefix,
# case-insensitive fallback; options may stand before the name); SQL keyword
# rules ignore case and do not fire on read-only searches (grep, rg, ag,
# ack), on git (git grep/log -S, commit messages), or on the cat/echo/printf
# that feeds one of those. DELETE without WHERE and TRUNCATE without TABLE
# count only when the input runs SQL (BV_RE_SQL_RUNNER): elsewhere "delete
# from the cart" is English.
#
# This is a guard against accidental destruction, not a sandbox: a script
# written to disk and run later, or text piped into a shell, is not inspected.
# Nesting deeper than any real command (BV_MAX_NEST call frames) is refused
# instead of parsed, since bash would run out of stack and fail open.
#
# Rule syntax (BV_GENERIC_RULES below, BV_FRAMEWORK_RULES after the block),
# fields separated by "|" (so no field may contain one), lists by ",":
#   argv|<programs>|<leading words>|<condition>|<category>
#   console|<full command name>|<condition>|<category>
#   read|<reader programs>|<file names>|<category>
#   write|<file names>|<category>
# Programs and readers are plain names; every other list item is a glob.
# Conditions: empty (always), has:<globs> (an argument matches), lacks:<globs>,
# arg:<globs> (the first positional argument matches), argsub:<names> (no
# positional argument, or one that is a substring of a listed name). For a
# console rule, has/lacks also see options before the command name and a
# "-x" item matches inside a short-option cluster; arg skips the value of
# BV_CONSOLE_VALUES options and BV_CONSOLE_SHORT_VALUES letters. A file name
# item "!glob" excludes; reader "sed-print" is sed without -i; a search
# reader (grep, rg, ...) skips its pattern operand and passes with -q, -l,
# -L or -c, which print no file content.
# Every block names its category; the command body is never printed.

BV_US=$'\037'
BV_RS=$'\036'
BV_GS=$'\035'
BV_NL=$'\n'
BV_TAB=$'\t'
BV_CR=$'\r'
BV_MAX_DEPTH=8
BV_MAX_NEST=100
BV_SEARCH=',grep,egrep,fgrep,rg,ag,ack,git,'
# Programs that run a command line given to them as one quoted word.
BV_LAUNCHERS=',ssh,sshpass,mosh,ddev,lando,vagrant,sail,docker,docker-compose,podman,podman-compose,nerdctl,kubectl,oc,gcloud,heroku,fly,flyctl,tmux,screen,watch,script,flock,parallel,nix-shell,devbox,distrobox,toolbox,multipass,limactl,'
# A command containing anything but these characters goes through the parser.
BV_NOT_PLAIN=$'*[!]A-Za-z0-9_./:=@%+,~^!?*{}[;&| \t-]*'
BV_RE_ASSIGN='^[A-Za-z_][A-Za-z0-9_]*[+]?='
BV_RE_BROAD='^(/[^/]*|~[A-Za-z0-9._-]*|(\.\.?/)*\.\.?)$'
BV_SQL_WS="[[:space:]${BV_US}]+"
BV_RE_SQL_DROP="(^|[^[:alnum:]_])[Dd][Rr][Oo][Pp]${BV_SQL_WS}([Tt][Aa][Bb][Ll][Ee]|[Dd][Aa][Tt][Aa][Bb][Aa][Ss][Ee]|[Ss][Cc][Hh][Ee][Mm][Aa])([^[:alnum:]_]|\$)"
BV_RE_SQL_TRUNCATE="(^|[^[:alnum:]_])[Tt][Rr][Uu][Nn][Cc][Aa][Tt][Ee]${BV_SQL_WS}[Tt][Aa][Bb][Ll][Ee]([^[:alnum:]_]|\$)"
# TRUNCATE without TABLE (optional in MySQL and PostgreSQL): a table name
# that ends the statement or is followed by a TRUNCATE option, not prose.
BV_RE_SQL_TRUNCATE_BARE="(^|[^[:alnum:]_])[Tt][Rr][Uu][Nn][Cc][Aa][Tt][Ee]${BV_SQL_WS}([Oo][Nn][Ll][Yy]${BV_SQL_WS})?[\"\`[:alpha:]_][][\"\`[:alnum:]_.\$]*([[:space:]]*(;|,|${BV_US}|\$)|${BV_SQL_WS}([Cc][Aa][Ss][Cc][Aa][Dd][Ee]|[Rr][Ee][Ss][Tt][Rr][Ii][Cc][Tt]|[Rr][Ee][Ss][Tt][Aa][Rr][Tt]|[Cc][Oo][Nn][Tt][Ii][Nn][Uu][Ee])([^[:alnum:]_]|\$))"
BV_RE_SQL_DELETE="(^|[^[:alnum:]_])[Dd][Ee][Ll][Ee][Tt][Ee]${BV_SQL_WS}[Ff][Rr][Oo][Mm]${BV_SQL_WS}([^;[:space:]${BV_US}][^;]*)"
BV_RE_SQL_WHERE="(^|[^[:alnum:]_])[Ww][Hh][Ee][Rr][Ee]([^[:alnum:]_]|\$)"
BV_RE_SQL_WHERE_ALL="[Ww][Hh][Ee][Rr][Ee]${BV_SQL_WS}(1[[:space:]]*=[[:space:]]*1|[Tt][Rr][Uu][Ee])([^[:alnum:]_]|\$)|[Ww][Hh][Ee][Rr][Ee]${BV_SQL_WS}1[[:space:]]*(${BV_US}|\$)"
# A word of a simple command that runs SQL text: an SQL client, wp db
# query/cli, artisan db/tinker, dbal:run-sql or doctrine:query:sql.
BV_SQL_NAME="[^${BV_US}[:space:]]"
BV_RE_SQL_RUNNER="${BV_US}(${BV_SQL_NAME}*/)?(mysql|mariadb|mysqlsh|mycli|psql|pgcli|sqlite3|sqlcmd|usql|tinker|artisan${BV_US}db|${BV_SQL_NAME}*:run-sql|${BV_SQL_NAME}*:query:sql)(${BV_US}|\$)|${BV_US}db${BV_US}(query|cli)(${BV_US}|\$)"

BV_GENERIC_RULES=(
  "argv|gh|repo delete||destructive command - gh repo delete"
  "argv|gh|repo archive||destructive command - gh repo archive"
  "argv|gh|issue delete||destructive command - gh issue delete"
  "argv|gh|release delete||destructive command - gh release delete"
  "argv|composer|config|has:github-oauth*,gitlab-oauth*,gitlab-token*,bitbucket-oauth*,forgejo-token*,http-basic*,bearer*,custom-headers*|secret exposure - composer config writing an auth token"
  "argv|dropdb|||destructive command - dropdb"
  "argv|mysqladmin||has:drop|destructive command - mysqladmin drop"
  "read|cat,tac,nl,head,tail,less,more,bat,batcat,sed-print,grep,egrep,fgrep,rg,ag,ack,awk,cut|.env,.env.local,.env.*.local|secret exposure - reading a .env file"
)

bv_hit() { [[ -n $BV_HIT ]] || BV_HIT=$1; }

# ---- parser ---------------------------------------------------------------
# bv_parse STOP DEPTH PARENT walks BV_S from BV_P. STOP is "" (end of text),
# ")" or "`". Segments are appended to the BV_SEG_* arrays; the helpers below
# work on bv_parse's locals (word, have, want, words, docs, seg). A long word
# or word list is kept in two parts (wordh/word, wordsh/words) so that each
# append copies at most a short tail instead of everything collected so far.
#
# Characters are read through BV_WIN, the text of BV_S from BV_WB to BV_WE:
# every ${BV_S:offset:length} copies the whole of BV_S, so reading a long
# command from it a character at a time would be quadratic. A loop moves the
# window (bv_win) before it can come within reach of its end.

bv_win() {
  BV_WIN=${BV_S:BV_P:1024}
  BV_WB=$BV_P; BV_WE=$(( BV_P + ${#BV_WIN} ))
}

# BV_SLICE: the text of BV_S from offset $1 to BV_P.
bv_slice() {
  if (( $1 >= BV_WB && BV_P <= BV_WE )); then
    BV_SLICE=${BV_WIN:$1-BV_WB:BV_P-$1}
  else
    BV_SLICE=${BV_S:$1:BV_P-$1}
  fi
}

# BV_UPTO: the text from BV_P up to the first $1 (or to the end; false then).
# The window is tried first: copying the whole remainder for every token
# makes long commands quadratic.
bv_upto() {
  local w
  (( BV_P + 512 <= BV_WE || BV_WE >= BV_L )) || bv_win
  w=${BV_WIN:BV_P-BV_WB}
  if [[ $w == *"$1"* ]]; then BV_UPTO=${w%%"$1"*}; return 0; fi
  (( BV_WE >= BV_L )) || w=${BV_S:BV_P}
  BV_UPTO=${w%%"$1"*}
  [[ $w == *"$1"* ]]
}

bv_new_seg() {
  BV_SEG=${#BV_SEG_WORDS[@]}
  BV_SEG_WORDS[BV_SEG]=''
  BV_SEG_REDIR[BV_SEG]=''
  BV_SEG_DOC[BV_SEG]=''
  BV_SEG_PROG[BV_SEG]=''
  BV_SEG_DEPTH[BV_SEG]=$1
  BV_SEG_PARENT[BV_SEG]=$2
}

bv_word() {
  (( have )) || return 0
  [[ -z $wordh ]] || { word=$wordh$word; wordh=''; }
  case $want in
    '')
      words=$words$BV_US$word
      (( ${#words} < 1024 )) || { wordsh=$wordsh$words; words=''; } ;;
    '<<'|'<<-') docs=$docs$BV_RS$seg$BV_GS$want$BV_GS$word ;;
    '<<<') BV_SEG_DOC[seg]=${BV_SEG_DOC[seg]}$BV_US$word ;;
    *) BV_SEG_REDIR[seg]=${BV_SEG_REDIR[seg]}$BV_US$want$BV_GS$word ;;
  esac
  word=''; have=0; want=''
}

bv_end_seg() {
  BV_SEG_WORDS[seg]=$wordsh$words
  wordsh=''; words=''; want=''
}

bv_heredocs() {
  local list=$docs$BV_RS entry owner op delim rest line body part
  docs=''
  list=${list#"$BV_RS"}
  while [[ -n $list ]]; do
    entry=${list%%"$BV_RS"*}; list=${list#*"$BV_RS"}
    owner=${entry%%"$BV_GS"*}; entry=${entry#*"$BV_GS"}
    op=${entry%%"$BV_GS"*}; delim=${entry#*"$BV_GS"}
    body=''
    rest=${BV_S:BV_P}
    # Fast path: a plain << body ends at the first line that is the delimiter.
    if [[ $op == '<<' && $BV_NL$rest == *"$BV_NL$delim$BV_NL"* && $rest != *"$BV_CR"* ]]; then
      if [[ $rest == "$delim$BV_NL"* ]]; then
        BV_P=$(( BV_P + ${#delim} + 1 ))
      else
        body=${rest%%"$BV_NL$delim$BV_NL"*}$BV_NL
        BV_P=$(( BV_P + ${#body} + ${#delim} + 1 ))
      fi
      BV_SEG_DOC[owner]=${BV_SEG_DOC[owner]}$BV_US$body
      continue
    fi
    part=''
    while (( BV_P < BV_L )); do
      bv_upto "$BV_NL"
      line=$BV_UPTO
      BV_P=$(( BV_P + ${#line} + 1 ))
      line=${line%"$BV_CR"}
      if [[ $op == '<<-' ]]; then
        while [[ $line == "$BV_TAB"* ]]; do line=${line#"$BV_TAB"}; done
      fi
      [[ $line == "$delim" ]] && break
      part=$part$line$BV_NL
      (( ${#part} < 1024 )) || { body=$body$part; part=''; }
    done
    BV_SEG_DOC[owner]=${BV_SEG_DOC[owner]}$BV_US$body$part
  done
}

bv_dollar() {
  local depth=$1 seg=$2 indq=$3 c2 c3 start rest chunk
  (( BV_P + 300 <= BV_WE || BV_WE >= BV_L )) || bv_win
  c2=${BV_WIN:BV_P-BV_WB+1:1}
  case $c2 in
    '(')
      start=$BV_P
      if [[ ${BV_WIN:BV_P-BV_WB+2:1} == '(' ]]; then
        bv_upto '))'
        BV_P=$(( BV_P + ${#BV_UPTO} + 2 ))
      else
        BV_P=$(( BV_P + 2 ))
        bv_parse ')' $(( depth + 1 )) "$seg"
      fi
      bv_slice "$start"; word=$word$BV_SLICE ;;
    '{')
      bv_upto '}'
      BV_P=$(( BV_P + ${#BV_UPTO} + 1 ))
      word=$word$BV_UPTO'}' ;;
    "'")
      if (( indq )); then
        word=$word'$'; BV_P=$(( BV_P + 1 ))
      else
        BV_P=$(( BV_P + 2 ))
        while (( BV_P < BV_L )); do
          (( BV_P + 300 <= BV_WE || BV_WE >= BV_L )) || bv_win
          (( ${#word} < 1024 )) || { wordh=$wordh$word; word=''; }
          c3=${BV_WIN:BV_P-BV_WB:1}
          case $c3 in
            "'") BV_P=$(( BV_P + 1 )); break ;;
            '\')
              c3=${BV_WIN:BV_P-BV_WB+1:1}
              case $c3 in n) c3=$BV_NL ;; t) c3=$BV_TAB ;; esac
              BV_P=$(( BV_P + 2 )); word=$word$c3 ;;
            *)
              rest=${BV_WIN:BV_P-BV_WB:256}
              chunk=${rest%%[\'\\]*}
              word=$word$chunk; BV_P=$(( BV_P + ${#chunk} )) ;;
          esac
        done
      fi ;;
    '"')
      (( indq )) && word=$word'$'
      BV_P=$(( BV_P + 1 )) ;;
    *)
      word=$word'$'; BV_P=$(( BV_P + 1 )) ;;
  esac
}

bv_dquote() {
  local depth=$1 seg=$2 c c2 rest chunk start
  while (( BV_P < BV_L )); do
    (( BV_P + 300 <= BV_WE || BV_WE >= BV_L )) || bv_win
    (( ${#word} < 1024 )) || { wordh=$wordh$word; word=''; }
    c=${BV_WIN:BV_P-BV_WB:1}
    case $c in
      '"') BV_P=$(( BV_P + 1 )); return 0 ;;
      '\')
        c2=${BV_WIN:BV_P-BV_WB+1:1}
        case $c2 in
          '$'|'`'|'"'|'\') word=$word$c2 ;;
          "$BV_NL") ;;
          *) word=$word$c$c2 ;;
        esac
        BV_P=$(( BV_P + 2 )) ;;
      '$') bv_dollar "$depth" "$seg" 1 ;;
      '`')
        start=$BV_P; BV_P=$(( BV_P + 1 ))
        bv_parse '`' $(( depth + 1 )) "$seg"
        bv_slice "$start"; word=$word$BV_SLICE ;;
      *)
        rest=${BV_WIN:BV_P-BV_WB:256}
        chunk=${rest%%[\"\\\$\`]*}
        [[ -n $chunk ]] || chunk=$c
        word=$word$chunk; BV_P=$(( BV_P + ${#chunk} )) ;;
    esac
  done
}

bv_parse() {
  local stop=$1 depth=$2 parent=$3
  local seg word='' wordh='' have=0 words='' wordsh='' want='' docs='' c c2 rest chunk start
  # Every ( $( ` <( level costs call frames; far past any real command bash
  # would run out of stack and the hook would die without blocking.
  if (( ${#FUNCNAME[@]} > BV_MAX_NEST )); then
    bv_hit 'command too complex - nesting too deep to check'
    BV_P=$BV_L; return 0
  fi
  bv_new_seg "$depth" "$parent"; seg=$BV_SEG
  while (( BV_P < BV_L )); do
    (( BV_P + 300 <= BV_WE || BV_WE >= BV_L )) || bv_win
    (( ${#word} < 1024 )) || { wordh=$wordh$word; word=''; }
    c=${BV_WIN:BV_P-BV_WB:1}
    case $c in
      "$BV_NL")
        bv_word; bv_end_seg; BV_P=$(( BV_P + 1 ))
        [[ -z $docs ]] || bv_heredocs
        bv_new_seg "$depth" "$parent"; seg=$BV_SEG ;;
      ' '|"$BV_TAB"|"$BV_CR")
        (( ! have )) || bv_word
        BV_P=$(( BV_P + 1 )) ;;
      ';'|'&'|'|')
        c2=${BV_WIN:BV_P-BV_WB+1:1}
        bv_word
        if [[ $c$c2 == '&>' ]]; then
          BV_P=$(( BV_P + 2 )); want='>'
          [[ ${BV_WIN:BV_P-BV_WB:1} != '>' ]] || BV_P=$(( BV_P + 1 ))
          continue
        fi
        bv_end_seg; BV_P=$(( BV_P + 1 ))
        case $c$c2 in '&&'|'||'|'|&'|';;'|';&') BV_P=$(( BV_P + 1 )) ;; esac
        bv_new_seg "$depth" "$parent"; seg=$BV_SEG ;;
      '(')
        if (( have )); then
          word=$word$c; BV_P=$(( BV_P + 1 ))
        else
          bv_end_seg; BV_P=$(( BV_P + 1 ))
          bv_parse ')' "$depth" "$parent"
          bv_new_seg "$depth" "$parent"; seg=$BV_SEG
        fi ;;
      ')')
        bv_word; bv_end_seg; BV_P=$(( BV_P + 1 ))
        [[ $stop != ')' ]] || return 0
        bv_new_seg "$depth" "$parent"; seg=$BV_SEG ;;
      '`')
        if [[ $stop == '`' ]]; then
          bv_word; bv_end_seg; BV_P=$(( BV_P + 1 )); return 0
        fi
        start=$BV_P; BV_P=$(( BV_P + 1 ))
        bv_parse '`' $(( depth + 1 )) "$seg"
        bv_slice "$start"; word=$word$BV_SLICE; have=1 ;;
      '<'|'>')
        c2=${BV_WIN:BV_P-BV_WB+1:1}
        if [[ $c2 == '(' ]]; then
          bv_word
          start=$BV_P; BV_P=$(( BV_P + 2 ))
          bv_parse ')' $(( depth + 1 )) "$seg"
          bv_slice "$start"; word=$BV_SLICE; have=1
          continue
        fi
        # An all-digit word glued to the operator is a file descriptor.
        if (( have )) && [[ -z $wordh && $word =~ ^[0-9]+$ ]]; then word=''; have=0; else bv_word; fi
        BV_P=$(( BV_P + 1 )); want=$c
        case $c$c2 in
          '<<')
            BV_P=$(( BV_P + 1 )); want='<<'
            case ${BV_WIN:BV_P-BV_WB:1} in
              '<') BV_P=$(( BV_P + 1 )); want='<<<' ;;
              '-') BV_P=$(( BV_P + 1 )); want='<<-' ;;
            esac ;;
          '>>') BV_P=$(( BV_P + 1 )); want='>>' ;;
          '>&'|'>|'|'<&'|'<>') BV_P=$(( BV_P + 1 )) ;;
        esac ;;
      "'")
        BV_P=$(( BV_P + 1 ))
        bv_upto "'"
        word=$word$BV_UPTO; have=1; BV_P=$(( BV_P + ${#BV_UPTO} + 1 )) ;;
      '"')
        BV_P=$(( BV_P + 1 )); have=1
        bv_dquote "$depth" "$seg" ;;
      '\')
        c2=${BV_WIN:BV_P-BV_WB+1:1}
        if [[ $c2 != "$BV_NL" ]]; then word=$word$c2; have=1; fi
        BV_P=$(( BV_P + 2 )) ;;
      '$')
        have=1
        bv_dollar "$depth" "$seg" 0 ;;
      '#')
        if (( have )); then
          word=$word$c; BV_P=$(( BV_P + 1 ))
        else
          bv_upto "$BV_NL"
          BV_P=$(( BV_P + ${#BV_UPTO} ))
        fi ;;
      *)
        rest=${BV_WIN:BV_P-BV_WB:256}
        chunk=${rest%%[[:space:]\;\&\|\(\)\<\>\'\"\\\$\`\#]*}
        [[ -n $chunk ]] || chunk=$c
        word=$word$chunk; have=1; BV_P=$(( BV_P + ${#chunk} )) ;;
    esac
  done
  bv_word; bv_end_seg
  return 0
}

bv_run_parse() {
  local s=$1 line
  case $s in
    *"$BV_US"*|*"$BV_RS"*|*"$BV_GS"*)
      s=${s//"$BV_US"/ }; s=${s//"$BV_RS"/ }; s=${s//"$BV_GS"/ } ;;
  esac
  # Fast path: without quotes, expansions, redirections, grouping, comments
  # or newlines, splitting on ; & | and blanks gives the parser's segments.
  # shellcheck disable=SC2053 # BV_NOT_PLAIN is a pattern
  if (( ${#s} <= 4096 )) && [[ $s != $BV_NOT_PLAIN ]]; then
    s=${s//';'/$BV_NL}; s=${s//'&'/$BV_NL}; s=${s//'|'/$BV_NL}$BV_NL
    while [[ -n $s ]]; do
      line=${s%%"$BV_NL"*}; s=${s:${#line}+1}
      [[ $line == *[![:blank:]]* ]] || continue
      bv_new_seg "$2" "$3"
      bv_split_words "$line"
    done
    return 0
  fi
  BV_S=$s; BV_P=0; BV_L=${#s}
  bv_win
  bv_parse '' "$2" "$3"
}

bv_split_words() {
  local IFS=$' \t' ws
  set -f
  # shellcheck disable=SC2206 # deliberate split on blanks, globbing off
  ws=($1)
  set +f
  IFS=$BV_US
  BV_SEG_WORDS[BV_SEG]=$BV_US${ws[*]}
}

# ---- normaliser -----------------------------------------------------------

bv_load_words() {
  local list=${BV_SEG_WORDS[$1]#"$BV_US"} IFS=$BV_US
  BV_W=()
  set -f
  # shellcheck disable=SC2206 # deliberate split on the word separator, globbing off
  [[ -z $list ]] || BV_W=($list)
  set +f
  BV_NW=${#BV_W[@]}
}

# Advance the caller's i past a wrapper's own options.
bv_skip_wrapper() {
  local w takes=''
  case $1 in
    command) case ${BV_W[i]-} in -v|-V) i=$BV_NW; return 0 ;; esac ;;
    sudo) takes=' -u -g -h -p -C -D -r -t -T -U -R ' ;;
    doas) takes=' -u -C ' ;;
    env) takes=' -u -C -S ' ;;
    nice) takes=' -n ' ;;
    ionice) takes=' -c -n -p -P -u ' ;;
    timeout) takes=' -s -k ' ;;
    stdbuf) takes=' -i -o -e ' ;;
    xargs) takes=' -I -n -P -L -s -d -E -a ' ;;
    time) takes=' -f -o ' ;;
    exec) takes=' -a ' ;;
  esac
  while (( i < BV_NW )); do
    w=${BV_W[i]}
    case $w in
      --) i=$(( i + 1 )); break ;;
      -) i=$(( i + 1 )) ;;
      -?*) if [[ $takes == *" $w "* ]]; then i=$(( i + 2 )); else i=$(( i + 1 )); fi ;;
      *) break ;;
    esac
  done
  [[ $1 != timeout ]] || i=$(( i + 1 ))
  return 0
}

# BV_SCRIPT: index of the script `php` at index $1 - 1 runs (false for -r etc.).
bv_php_script() {
  local j=$1
  BV_SCRIPT=-1
  while (( j < BV_NW )); do
    case ${BV_W[j]} in
      -r|-R|-B|-E|-a|-i|-l|-m|-s|-v|-w|-h|--run|--info) return 1 ;;
      -f|-F) j=$(( j + 1 )); break ;;
      -c|-d|-S|-t|-z|--rf|--rc|--re|--rz|--ri) j=$(( j + 2 )) ;;
      --) j=$(( j + 1 )); break ;;
      -*) j=$(( j + 1 )) ;;
      *) break ;;
    esac
  done
  (( j < BV_NW )) || return 1
  BV_SCRIPT=$j
}

# BV_PI/BV_PROG/BV_AI: index, normalised name and first argument of the program.
bv_locate_program() {
  local i=0 w b
  while (( i < BV_NW )); do
    w=${BV_W[i]}
    case $w in
      [A-Za-z_]*=*) if [[ $w =~ $BV_RE_ASSIGN ]]; then i=$(( i + 1 )); continue; fi ;;
    esac
    bv_base "$w"; b=$BV_BASE
    case $b in
      '!'|'{'|'}'|if|then|elif|else|do|while|until|builtin|nohup) i=$(( i + 1 )) ;;
      command|exec|sudo|doas|env|nice|ionice|timeout|stdbuf|xargs|time)
        i=$(( i + 1 )); bv_skip_wrapper "$b" ;;
      *) break ;;
    esac
  done
  BV_PI=$i; BV_AI=$(( i + 1 )); BV_PROG=''
  (( i < BV_NW )) || { BV_AI=$BV_NW; return 0; }
  bv_base "${BV_W[i]}"; BV_PROG=$BV_BASE
  case $BV_PROG in
    php|php[0-9]*)
      if bv_php_script "$BV_AI"; then
        bv_base "${BV_W[BV_SCRIPT]}"; BV_PROG=$BV_BASE; BV_AI=$(( BV_SCRIPT + 1 ))
      fi ;;
  esac
  BV_PROG=${BV_PROG%.phar}
  [[ $BV_PROG != wp-cli ]] || BV_PROG=wp
  return 0
}

# ---- helpers --------------------------------------------------------------

# BV_BASE: $1 without its directory. ${w##*/} is quadratic in the word's
# length, and no program or file name is longer than this cap.
bv_base() {
  case $1 in
    */*) if (( ${#1} <= 512 )); then BV_BASE=${1##*/}; else BV_BASE=''; fi ;;
    *) BV_BASE=$1 ;;
  esac
}

# True when $1 matches a glob of the comma list $2 and no "!glob" item.
bv_glob_in() {
  local v=$1 list=$2, g hit=1
  while [[ -n $list ]]; do
    g=${list%%,*}; list=${list#*,}
    # shellcheck disable=SC2053 # list items are patterns
    case $g in
      '!'*) [[ $v != ${g#!} ]] || return 1 ;;
      *) [[ $v != $g ]] || hit=0 ;;
    esac
  done
  return $hit
}

# True when $1 is a short-option cluster (-abc) that sets option $2 before a
# letter of $3 takes the rest of the cluster as its value.
bv_short() {
  local w=$1 k c
  case $w in --*|-|[!-]*|'') return 1 ;; esac
  (( ${#w} <= 64 )) || return 1
  for (( k = 1; k < ${#w}; k++ )); do
    c=${w:k:1}
    [[ $c != "$2" ]] || return 0
    [[ -z ${3-} || $3 != *"$c"* ]] || return 1
  done
  return 1
}

# True when the short-option cluster $1 ends in a letter of $2 whose value is
# the next word.
bv_short_value() {
  local w=$1 k c
  case $w in --*|-|[!-]*|'') return 1 ;; esac
  (( ${#w} <= 64 )) || return 1
  for (( k = 1; k < ${#w}; k++ )); do
    c=${w:k:1}
    if [[ $2 == *"$c"* ]]; then (( k == ${#w} - 1 )); return; fi
  done
  return 1
}

# BV_NEXT: the index after console option $1 and, when that option takes the
# next word as its value (BV_CONSOLE_VALUES, BV_CONSOLE_SHORT_VALUES), after
# the value too. Symfony Console gives a value-taking option the next word
# unless it starts with "-".
bv_console_opt() {
  local w=${BV_W[$1]}
  BV_NEXT=$(( $1 + 1 ))
  [[ $w != *=* && ${BV_W[BV_NEXT]-} != -* ]] || return 0
  if bv_glob_in "$w" "${BV_CONSOLE_VALUES---env}" ||
     bv_short_value "$w" "${BV_CONSOLE_SHORT_VALUES-e}"; then
    BV_NEXT=$(( BV_NEXT + 1 ))
  fi
  return 0
}

# True when the short-option cluster $1 sets a letter named by a "-x" item
# of the list $2 before a letter that takes the rest of the cluster.
bv_short_in() {
  local list=$2, g
  while [[ -n $list ]]; do
    g=${list%%,*}; list=${list#*,}
    [[ $g != -[!-] ]] || ! bv_short "$1" "${g#-}" "${BV_CONSOLE_SHORT_VALUES-e}" || return 0
  done
  return 1
}

bv_lower() {
  BV_LOWER=$1
  case $1 in *[A-Z]*) ;; *) return 0 ;; esac
  local s=$1 out='' c k p up=ABCDEFGHIJKLMNOPQRSTUVWXYZ lo=abcdefghijklmnopqrstuvwxyz
  for (( k = 0; k < ${#s}; k++ )); do
    c=${s:k:1}
    case $c in [A-Z]) p=${up%%"$c"*}; c=${lo:${#p}:1} ;; esac
    out=$out$c
  done
  BV_LOWER=$out
}

# Symfony Console resolution: same number of ":" parts, each typed part (in
# lower case) a prefix of the full one.
bv_console_name() {
  local t=$1 f=$2 tp fp
  while :; do
    tp=${t%%:*}; fp=${f%%:*}
    [[ $fp == "$tp"* ]] || return 1
    if [[ $t == *:* ]]; then
      [[ $f == *:* ]] || return 1
      t=${t#*:}; f=${f#*:}
    else
      [[ $f != *:* ]]; return
    fi
  done
}

# BV_CON_NAME/BV_CON_I: the command name (lower case) passed to a console entry
# point (BV_CONSOLE_ENTRIES, `sail art|a` where artisan is one, or any
# `php script` when BV_CONSOLE_PHP_SCRIPTS=1); BV_CON_A: the entry point's
# first argument.
bv_console_locate() {
  local j=$BV_PI w k any
  BV_CON_NAME=''; BV_CON_I=-1; BV_CON_A=-1
  [[ -n $BV_CONSOLE_ENTRIES && $BV_SEARCH,echo,printf, != *",$BV_PROG,"* ]] || return 0
  # Cheap pre-check: without an entry point, php or sail in the words, nothing to find.
  w=${BV_SEG_WORDS[BV_CUR]}
  if [[ $w != *php* && $w != *sail* ]]; then
    k=$BV_CONSOLE_ENTRIES,
    # shellcheck disable=SC2053 # entries are patterns
    while [[ -n $k && $w != *${k%%,*}* ]]; do k=${k#*,}; done
    [[ -n $k ]] || return 0
  fi
  while (( j < BV_NW )); do
    w=${BV_W[j]}; k=-1; any=0
    case $w in *[[:space:]]*) j=$(( j + 1 )); continue ;; esac
    bv_base "$w"
    if bv_glob_in "${BV_BASE%.phar}" "$BV_CONSOLE_ENTRIES"; then
      k=$(( j + 1 ))
    elif [[ $BV_BASE == sail ]]; then
      # Sail runs `art` and `a` as artisan.
      case ${BV_W[j+1]-} in
        art|a) ! bv_glob_in artisan "$BV_CONSOLE_ENTRIES" || k=$(( j + 2 )) ;;
      esac
    elif [[ $BV_BASE == php || $BV_BASE == php[0-9]* ]] && bv_php_script $(( j + 1 )); then
      bv_base "${BV_W[BV_SCRIPT]}"; w=${BV_BASE%.phar}; w=${w%.php}
      if bv_glob_in "$w" "$BV_CONSOLE_ENTRIES"; then
        k=$(( BV_SCRIPT + 1 ))
      elif (( BV_CONSOLE_PHP_SCRIPTS )); then
        k=$(( BV_SCRIPT + 1 )); any=1
      fi
      j=$BV_SCRIPT
    fi
    if (( k >= 0 )); then
      BV_CON_A=$k
      while (( k < BV_NW )); do
        case ${BV_W[k]} in
          -*) bv_console_opt "$k"; k=$BV_NEXT ;;
          *) break ;;
        esac
      done
      if (( k < BV_NW && ${#BV_W[k]} <= 128 )); then
        bv_lower "${BV_W[k]}"
        BV_CON_HEAD=${BV_LOWER%%:*}; BV_CON_I=$k
        # Keep the name only if a console rule's first part can start with
        # it. A script that is not a listed entry (phpunit, pest, ...) counts
        # only with a namespaced name, so `--filter rollback` is no command.
        if [[ $BV_CONSOLE_HEADS == *",$BV_CON_HEAD"* ]] &&
           { (( ! any )) || [[ $BV_LOWER == *:* ]]; }; then
          BV_CON_NAME=$BV_LOWER
        fi
      fi
      return 0
    fi
    j=$(( j + 1 ))
  done
  return 0
}

# BV_LEAD_END: index after the positional words $1 (space-separated) that
# must follow the program; options and @aliases between them are skipped.
bv_lead() {
  local lead="$1 " w j=$BV_AI
  while [[ -n ${lead// /} ]]; do
    w=${lead%% *}; lead=${lead#* }
    while (( j < BV_NW )); do
      case ${BV_W[j]} in -*|@*) j=$(( j + 1 )) ;; *) break ;; esac
    done
    [[ ${BV_W[j]-} == "$w" ]] || return 1
    j=$(( j + 1 ))
  done
  BV_LEAD_END=$j
}

# bv_cond CONDITION FIRST_ARGUMENT FIRST_POSITIONAL [COMMAND_NAME_INDEX]
# With the fourth argument the words are a console's: has/lacks skip the
# command name and also match "-x" items inside a short-option cluster, and
# arg steps over option values.
bv_cond() {
  local type=${1%%:*} list=${1#*:} con=${4--1} j w found=1 name names
  case $type in
    '') return 0 ;;
    has|lacks)
      for (( j = $2; j < BV_NW; j++ )); do
        (( j != con )) || continue
        w=${BV_W[j]}
        [[ $w != -- ]] || break
        if bv_glob_in "$w" "$list"; then found=0; break; fi
        if (( con >= 0 )) && [[ $w == -[!-]* ]] && bv_short_in "$w" "$list"; then found=0; break; fi
      done
      if [[ $type == has ]]; then return $found; fi
      (( found )) ;;
    arg)
      j=$3
      while (( j < BV_NW )); do
        case ${BV_W[j]} in
          -*) if (( con >= 0 )); then bv_console_opt "$j"; j=$BV_NEXT; else j=$(( j + 1 )); fi ;;
          *) break ;;
        esac
      done
      (( j < BV_NW )) && bv_glob_in "${BV_W[j]}" "$list" ;;
    argsub)
      for (( j = $3; j < BV_NW; j++ )); do
        w=${BV_W[j]}
        case $w in -*) continue ;; esac
        found=0; names=$list,
        while [[ -n $names ]]; do
          name=${names%%,*}; names=${names#*,}
          [[ $name != *"$w"* ]] || return 0
        done
      done
      (( found )) ;;
    *) return 1 ;;
  esac
}

# True when a redirection of the current segment with an operator in the
# space-separated list $1 targets a file name matching $2.
bv_redirects() {
  local list=${BV_SEG_REDIR[BV_CUR]#"$BV_US"} e op t
  [[ -n $list ]] || return 1
  list=$list$BV_US
  while [[ -n $list ]]; do
    e=${list%%"$BV_US"*}; list=${list#*"$BV_US"}
    op=${e%%"$BV_GS"*}; t=${e#*"$BV_GS"}
    [[ " $1 " == *" $op "* ]] || continue
    bv_base "$t"
    ! bv_glob_in "$BV_BASE" "$2" || return 0
  done
  return 1
}

# bv_reads READERS FILES: the program prints a file named in FILES.
bv_reads() {
  local j w pat=0
  if [[ $BV_PROG == sed && ",$1," == *,sed-print,* ]]; then
    # In place (-i, -Ei, -ni, but not -ei: there i is the script) prints nothing.
    for (( j = BV_AI; j < BV_NW; j++ )); do
      case ${BV_W[j]} in -i*|--in-place*) return 1 ;; esac
      ! bv_short "${BV_W[j]}" i efl || return 1
    done
  elif ! bv_glob_in "$BV_PROG" "$1"; then
    return 1
  fi
  # A search's first operand is its pattern unless -e or -f gave one; with
  # -q, -l, -L or -c it prints no line of the file.
  if [[ $BV_SEARCH == *",$BV_PROG,"* ]]; then
    pat=1
    for (( j = BV_AI; j < BV_NW; j++ )); do
      case ${BV_W[j]} in
        --) break ;;
        -q|-l|-L|-c|--quiet|--silent|--count|--files-with-matches|--files-without-match) return 1 ;;
        -e*|-f*|--regexp|--regexp=*|--file|--file=*) pat=0 ;;
      esac
    done
  fi
  for (( j = BV_AI; j < BV_NW; j++ )); do
    w=${BV_W[j]}
    case $w in -?*) continue ;; esac
    if (( pat )); then pat=0; continue; fi
    bv_base "$w"
    ! bv_glob_in "$BV_BASE" "$2" || return 0
  done
  bv_redirects '<' "$2"
}

# bv_writes FILES: output redirection (or tee) onto a file named in FILES.
bv_writes() {
  local j w
  if [[ $BV_PROG == tee ]]; then
    for (( j = BV_AI; j < BV_NW; j++ )); do
      w=${BV_W[j]}
      case $w in -?*) continue ;; esac
      bv_base "$w"
      ! bv_glob_in "$BV_BASE" "$1" || return 0
    done
  fi
  bv_redirects '> >>' "$1"
}

# Walk the rules only when one can apply to the segment: an argv rule names
# the program, a read rule lists it (or sed) and it has a file to read, a
# console command name was found, or output is redirected (or tee'd).
bv_apply_rules() {
  local rule a b c cat argv=0 read=0 write=0
  if [[ -n $BV_PROG ]]; then
    [[ $BV_ARGV_PROGS != *",$BV_PROG,"* ]] || argv=1
    if (( BV_AI < BV_NW )) || [[ ${BV_SEG_REDIR[BV_CUR]} == *'<'* ]]; then
      [[ $BV_READERS != *",$BV_PROG,"* && $BV_PROG != sed ]] || read=1
    fi
  fi
  [[ $BV_PROG != tee && ${BV_SEG_REDIR[BV_CUR]} != *'>'* ]] || write=1
  (( argv || read || write )) || [[ -n $BV_CON_NAME ]] || return 0
  for rule in ${BV_GENERIC_RULES[@]+"${BV_GENERIC_RULES[@]}"} \
    ${BV_FRAMEWORK_RULES[@]+"${BV_FRAMEWORK_RULES[@]}"}; do
    case $rule in
      'argv|'*)
        (( argv )) || continue
        rule=${rule#argv|}; a=${rule%%|*}
        [[ ",$a," == *",$BV_PROG,"* ]] || continue
        rule=${rule#*|}; b=${rule%%|*}; rule=${rule#*|}; c=${rule%%|*}; cat=${rule#*|}
        bv_lead "$b" || continue
        ! bv_cond "$c" "$BV_AI" "$BV_LEAD_END" || { bv_hit "$cat"; return 0; } ;;
      'console|'*)
        [[ -n $BV_CON_NAME ]] || continue
        rule=${rule#console|}; a=${rule%%|*}
        [[ ${a%%:*} == "$BV_CON_HEAD"* ]] || continue
        rule=${rule#*|}; c=${rule%%|*}; cat=${rule#*|}
        bv_console_name "$BV_CON_NAME" "$a" || continue
        ! bv_cond "$c" "$BV_CON_A" $(( BV_CON_I + 1 )) "$BV_CON_I" || { bv_hit "$cat"; return 0; } ;;
      'read|'*)
        (( read )) || continue
        rule=${rule#read|}; a=${rule%%|*}; rule=${rule#*|}; b=${rule%%|*}; cat=${rule#*|}
        ! bv_reads "$a" "$b" || { bv_hit "$cat"; return 0; } ;;
      'write|'*)
        (( write )) || continue
        rule=${rule#write|}; a=${rule%%|*}; cat=${rule#*|}
        ! bv_writes "$a" || { bv_hit "$cat"; return 0; } ;;
    esac
  done
  return 0
}

# BV_ARGV_PROGS/BV_READERS/BV_CONSOLE_HEADS: ",a,b," lists of the programs
# argv and read rules name (plain names, not globs) and of the first part of
# every console rule's command name.
bv_index_rules() {
  local rule
  BV_ARGV_PROGS=','; BV_READERS=','; BV_CONSOLE_HEADS=','
  for rule in ${BV_GENERIC_RULES[@]+"${BV_GENERIC_RULES[@]}"} \
    ${BV_FRAMEWORK_RULES[@]+"${BV_FRAMEWORK_RULES[@]}"}; do
    case $rule in
      'argv|'*) rule=${rule#argv|}; BV_ARGV_PROGS=$BV_ARGV_PROGS${rule%%|*}, ;;
      'read|'*) rule=${rule#read|}; BV_READERS=$BV_READERS${rule%%|*}, ;;
      'console|'*) rule=${rule#console|}; rule=${rule%%|*}; BV_CONSOLE_HEADS=$BV_CONSOLE_HEADS${rule%%:*}, ;;
    esac
  done
}

# ---- generic rules --------------------------------------------------------

bv_recurse() {
  local d=$(( BV_SEG_DEPTH[BV_CUR] + 1 ))
  [[ -n $1 ]] || return 0
  (( d <= BV_MAX_DEPTH )) || return 0
  bv_run_parse "$1" "$d" "$BV_CUR"
}

# sh/bash/... -c STRING, su -c STRING, a shell reading a heredoc or
# here-string on stdin (itself, or reached through a launcher), and ssh HOST
# with no remote command, which runs its stdin in the remote login shell.
bv_shell_scan() {
  local j=$BV_PI w k v cflag sflag launched=0
  [[ $BV_LAUNCHERS != *",$BV_PROG,"* ]] || launched=1
  while (( j < BV_NW )); do
    bv_base "${BV_W[j]}"; w=$BV_BASE
    j=$(( j + 1 ))
    case $w in
      su|runuser)
        for (( k = j; k < BV_NW; k++ )); do
          case ${BV_W[k]} in
            -c|--command) bv_recurse "${BV_W[k+1]-}"; break ;;
            --command=*) bv_recurse "${BV_W[k]#--command=}"; break ;;
          esac
        done ;;
      sh|bash|zsh|dash|ksh|ash|mksh|yash|fish)
        k=$j; cflag=0; sflag=0
        while (( k < BV_NW )); do
          w=${BV_W[k]}
          case $w in
            --rcfile|--init-file) k=$(( k + 2 )); continue ;;
            -|--) k=$(( k + 1 )); break ;;
            --command) cflag=1 ;;
            --*) ;;
            -?*|+?*)
              if [[ $w == -* ]]; then
                ! bv_short "$w" c || cflag=1
                ! bv_short "$w" s || sflag=1
              fi
              # Every o or O of a cluster (-euo pipefail) takes the next word.
              if (( ${#w} <= 64 )); then v=${w//[!oO]/}; k=$(( k + ${#v} )); fi ;;
            *) break ;;
          esac
          k=$(( k + 1 ))
        done
        if (( cflag )); then
          bv_recurse "${BV_W[k]-}"
        elif (( sflag || k >= BV_NW )) && (( j - 1 == BV_PI || launched )); then
          bv_recurse "${BV_SEG_DOC[BV_CUR]}"
        fi ;;
      ssh)
        (( j - 1 == BV_PI || launched )) || continue
        k=$j
        while (( k < BV_NW )); do
          w=${BV_W[k]}
          case $w in
            --) k=$(( k + 1 )); break ;;
            -?*) if bv_short_value "$w" BbcDEeFIiJLlmOoPpQRSWw; then k=$(( k + 2 )); else k=$(( k + 1 )); fi ;;
            *) break ;;
          esac
        done
        (( k + 1 < BV_NW )) || bv_recurse "${BV_SEG_DOC[BV_CUR]}" ;;
    esac
  done
  return 0
}

bv_rule_eval() {
  local j s=''
  for (( j = BV_AI; j < BV_NW; j++ )); do s="$s ${BV_W[j]}"; done
  bv_recurse "$s"
}

# A `git -c` or `--config-env` setting that disables the repository's hooks.
bv_git_config() {
  case $1 in
    [Cc][Oo][Rr][Ee].[Hh][Oo][Oo][Kk][Ss][Pp][Aa][Tt][Hh]=*)
      bv_hit 'verification bypass - git -c core.hooksPath' ;;
  esac
}

# `git config` writing or unsetting core.hooksPath turns the hooks off for
# every later commit, not just one. Reads (--get, get, --list) pass.
bv_git_config_set() {
  local j w key=0 val=0 rd=0 un=0
  for (( j = $1; j < BV_NW; j++ )); do
    w=${BV_W[j]}
    case $w in
      --get|--get-all|--get-regexp|--get-urlmatch|--list|-l|get|list) rd=1 ;;
      --unset|--unset-all|unset) un=1 ;;
      --file|-f|--blob|--type|--default|--comment|--value) j=$(( j + 1 )) ;;
      -*) ;;
      [Cc][Oo][Rr][Ee].[Hh][Oo][Oo][Kk][Ss][Pp][Aa][Tt][Hh]) key=1 ;;
      *) (( ! key )) || val=1 ;;
    esac
  done
  (( key && ! rd && ( val || un ) )) && bv_hit 'verification bypass - git config core.hooksPath'
  return 0
}

# git submodule [options] foreach [--recursive] COMMAND...: the command runs
# in a shell in every submodule.
bv_git_foreach() {
  local j=$1 s=''
  while (( j < BV_NW )); do
    case ${BV_W[j]} in
      foreach) j=$(( j + 1 )); break ;;
      -*) j=$(( j + 1 )) ;;
      *) return 0 ;;
    esac
  done
  while (( j < BV_NW )); do
    case ${BV_W[j]} in --recursive|-q|--quiet) j=$(( j + 1 )) ;; *) break ;; esac
  done
  for (( ; j < BV_NW; j++ )); do s="$s ${BV_W[j]}"; done
  bv_recurse "$s"
}

# bv_git_scan FROM LONG SHORT STOPS CATEGORY: LONG, or the short option
# SHORT in a cluster, appears before "--".
bv_git_scan() {
  local j w
  for (( j = $1; j < BV_NW; j++ )); do
    w=${BV_W[j]}
    [[ $w != -- ]] || return 0
    if [[ $w == "$2" ]] || { [[ -n $3 ]] && bv_short "$w" "$3" "$4"; }; then
      bv_hit "$5"; return 0
    fi
  done
  return 0
}

bv_git_push() {
  local j=$1 w opts=1
  while (( j < BV_NW )); do
    w=${BV_W[j]}; j=$(( j + 1 ))
    if (( opts )); then
      case $w in
        --) opts=0; continue ;;
        --force|--force-with-lease|--force-with-lease=*|--mirror)
          bv_hit 'destructive command - git force push'; return 0 ;;
        --repo|--receive-pack|--exec|--push-option|-o) j=$(( j + 1 )); continue ;;
        --*) continue ;;
        -?*)
          if bv_short "$w" f o; then bv_hit 'destructive command - git force push'; return 0; fi
          continue ;;
      esac
    fi
    case $w in +?*) bv_hit 'destructive command - git force push (+refspec)'; return 0 ;; esac
  done
  return 0
}

bv_git_commit() {
  local j=$1 w
  while (( j < BV_NW )); do
    w=${BV_W[j]}; j=$(( j + 1 ))
    case $w in
      --) return 0 ;;
      --no-verify) bv_hit 'verification bypass - git commit --no-verify'; return 0 ;;
      --message|--file|--author|--date|--template|--reuse-message|--reedit-message|--fixup|--squash|--cleanup|--trailer|--pathspec-from-file)
        j=$(( j + 1 )) ;;
      --*) ;;
      -?*)
        if bv_short "$w" n mFCctSu; then
          bv_hit 'verification bypass - git commit -n'; return 0
        fi
        ! bv_short_value "$w" mFCct || j=$(( j + 1 )) ;;
    esac
  done
  return 0
}

bv_git_branch() {
  local j w del=0 force=0
  for (( j = $1; j < BV_NW; j++ )); do
    w=${BV_W[j]}
    case $w in
      --) break ;;
      --delete) del=1 ;;
      --force) force=1 ;;
      --*) ;;
      -?*)
        if bv_short "$w" D; then del=1; force=1; fi
        ! bv_short "$w" d || del=1
        ! bv_short "$w" f || force=1 ;;
    esac
  done
  (( del && force )) && bv_hit 'destructive command - git branch force delete'
  return 0
}

bv_rule_git() {
  local j=$BV_AI w sub=''
  while (( j < BV_NW )); do
    w=${BV_W[j]}
    case $w in
      -c|--config-env) bv_git_config "${BV_W[j+1]-}"; j=$(( j + 2 )) ;;
      --config-env=*) bv_git_config "${w#--config-env=}"; j=$(( j + 1 )) ;;
      -C|--git-dir|--work-tree|--namespace|--super-prefix|--attr-source) j=$(( j + 2 )) ;;
      -*) j=$(( j + 1 )) ;;
      *) sub=$w; j=$(( j + 1 )); break ;;
    esac
  done
  [[ -z $BV_HIT && -n $sub ]] || return 0
  case $sub in
    push) bv_git_push "$j" ;;
    commit) bv_git_commit "$j"; return 0 ;;
    reset) bv_git_scan "$j" --hard '' '' 'destructive command - git reset --hard' ;;
    clean) bv_git_scan "$j" --force f e 'destructive command - git clean --force' ;;
    branch) bv_git_branch "$j" ;;
    config) bv_git_config_set "$j" ;;
    submodule) bv_git_foreach "$j" ;;
    # Read-only commands have no --no-verify: there it is a search value.
    log|grep|show|diff|shortlog|whatchanged|blame|rev-list|reflog) return 0 ;;
  esac
  [[ -z $BV_HIT ]] || return 0
  bv_git_scan "$j" --no-verify '' '' 'verification bypass - git --no-verify'
}

# Root, home and working-tree targets (/, /usr, ~, $HOME, ., .., *, ./*).
bv_broad_path() {
  local t=$1
  (( ${#t} <= 512 )) || return 1
  while [[ $t == ?*/ ]]; do t=${t%/}; done
  case $t in */\*) t=${t%/\*}; [[ -n $t ]] || t=/ ;; esac
  while [[ $t == ?*/ ]]; do t=${t%/}; done
  case $t in
    '*'|'.*'|'$HOME'|'${HOME}'|'$PWD'|'${PWD}'|'$(pwd)'|'`pwd`') return 0 ;;
  esac
  [[ $t =~ $BV_RE_BROAD ]]
}

bv_rule_rm() {
  local j=$BV_AI w rec=0 broad=0 opts=1
  while (( j < BV_NW )); do
    w=${BV_W[j]}; j=$(( j + 1 ))
    if (( opts )); then
      case $w in
        --) opts=0; continue ;;
        --no-preserve-root) bv_hit 'destructive command - rm --no-preserve-root'; return 0 ;;
        --recursive) rec=1; continue ;;
        --*) continue ;;
        -?*)
          if bv_short "$w" r || bv_short "$w" R; then rec=1; fi
          continue ;;
      esac
    fi
    ! bv_broad_path "$w" || broad=1
  done
  (( rec && broad )) && bv_hit 'destructive command - recursive rm of a root, home or working-tree path'
  return 0
}

bv_rule_gh() {
  local j w m='' first=''
  for (( j = BV_AI; j < BV_NW; j++ )); do
    w=${BV_W[j]}
    case $w in
      -X|--method) m=${BV_W[j+1]-}; j=$(( j + 1 )) ;;
      -X?*) m=${w#-X} ;;
      --method=*) m=${w#--method=} ;;
      -*) ;;
      *) [[ -n $first ]] || first=$w ;;
    esac
  done
  [[ $first == api ]] || return 0
  case $m in [Dd][Ee][Ll][Ee][Tt][Ee]) bv_hit 'destructive command - gh api DELETE' ;; esac
  return 0
}

# True when a simple command of the input runs SQL text (BV_RE_SQL_RUNNER).
bv_sql_runs() {
  local i=0 n=${#BV_SEG_WORDS[@]}
  while (( i < n )); do
    [[ ! ${BV_SEG_WORDS[i]} =~ $BV_RE_SQL_RUNNER ]] || return 0
    i=$(( i + 1 ))
  done
  return 1
}

# DROP and TRUNCATE TABLE and an always-true WHERE are refused wherever they
# appear; TRUNCATE without TABLE and DELETE with no WHERE at all only when
# the input runs SQL, since elsewhere they are usually English.
bv_rule_sql() {
  local t=$1 n=0 runs='' whole rest
  if [[ $t =~ $BV_RE_SQL_DROP ]]; then
    bv_hit 'destructive SQL - DROP TABLE/DATABASE/SCHEMA'; return 0
  fi
  if [[ $t =~ $BV_RE_SQL_TRUNCATE ]] || { [[ $t =~ $BV_RE_SQL_TRUNCATE_BARE ]] && bv_sql_runs; }; then
    bv_hit 'destructive SQL - TRUNCATE'; return 0
  fi
  while (( n < 16 )) && [[ $t =~ $BV_RE_SQL_DELETE ]]; do
    whole=${BASH_REMATCH[0]}; rest=${BASH_REMATCH[2]}
    if [[ $rest =~ $BV_RE_SQL_WHERE_ALL ]]; then
      bv_hit 'destructive SQL - DELETE without a row filter'; return 0
    fi
    if ! [[ $rest =~ $BV_RE_SQL_WHERE ]]; then
      if [[ -z $runs ]]; then if bv_sql_runs; then runs=1; else runs=0; fi; fi
      (( ! runs )) || { bv_hit 'destructive SQL - DELETE without a row filter'; return 0; }
    fi
    rest=${t%%"$whole"*}
    t=${t:${#rest}+${#whole}}; n=$(( n + 1 ))
  done
  return 0
}

# True when the validator has rules keyed on program $1.
bv_known_program() {
  case ,git,rm,gh,eval,echo,printf,sed,tee$BV_SEARCH$BV_ARGV_PROGS$BV_READERS in
    *",$1,"*) return 0 ;;
  esac
  return 1
}

# A launcher without rules of its own (docker compose exec app, ddev, lando,
# ssh host, ...) runs a later word as the command: make the first such word
# that has rules the program, so its rules see it. A quoted command line
# given to a known launcher (ssh host '...', ddev exec "...", vagrant ssh -c
# "...", gcloud compute ssh --command="...") is checked as a command.
bv_launched() {
  local k w rec=0
  [[ $BV_LAUNCHERS != *",$BV_PROG,"* ]] || rec=1
  for (( k = BV_PI + 1; k < BV_NW; k++ )); do
    w=${BV_W[k]}
    case $w in
      --*=*[[:space:]]*) (( ! rec )) || bv_recurse "${w#*=}"; continue ;;
      -*|'') continue ;;
      *[[:space:]]*)
        # An option's VAR=value (docker run -e "MSG=a b") is not a command.
        if (( rec )) && ! { [[ ${BV_W[k-1]} == -* ]] && [[ $w =~ $BV_RE_ASSIGN ]]; }; then
          bv_recurse "$w"
        fi
        continue ;;
      *=*) continue ;;
    esac
    bv_base "$w"; w=${BV_BASE%.phar}
    [[ $w != wp-cli ]] || w=wp
    if [[ -n $w ]] && bv_known_program "$w"; then
      BV_PI=$k; BV_PROG=$w; BV_AI=$(( k + 1 ))
      return 0
    fi
  done
  return 1
}

bv_program_rules() {
  case $BV_PROG in
    git) bv_rule_git ;;
    rm) bv_rule_rm ;;
    gh) bv_rule_gh ;;
    eval) bv_rule_eval ;;
  esac
}

# SQL text is data in a search, and in the cat/echo/printf that feeds one.
bv_sql_check() {
  local parent=${BV_SEG_PARENT[BV_CUR]}
  [[ $BV_SEARCH != *",$BV_PROG,"* ]] || return 0
  case $BV_PROG in
    cat|echo|printf)
      (( parent < 0 )) || [[ $BV_SEARCH != *",${BV_SEG_PROG[parent]},"* ]] || return 0 ;;
  esac
  bv_rule_sql "${BV_SEG_WORDS[BV_CUR]}${BV_SEG_DOC[BV_CUR]}"
}

bv_check_seg() {
  local words=${BV_SEG_WORDS[$1]}
  BV_CUR=$1
  [[ -n $words || -n ${BV_SEG_REDIR[$1]} || -n ${BV_SEG_DOC[$1]} ]] || return 0
  bv_load_words "$1"
  bv_locate_program
  BV_SEG_PROG[BV_CUR]=$BV_PROG
  bv_program_rules
  [[ -z $BV_HIT ]] || return 0
  if [[ $BV_SEARCH,echo,printf, != *",$BV_PROG,"* ]]; then
    case $words in *sh*|*su*|*runuser*) bv_shell_scan ;; esac
  fi
  bv_console_locate
  bv_apply_rules
  [[ -z $BV_HIT ]] || return 0
  if [[ -n $BV_PROG ]] && ! bv_known_program "$BV_PROG" && bv_launched; then
    BV_CON_NAME=''
    bv_program_rules
    [[ -z $BV_HIT ]] || return 0
    bv_apply_rules
    [[ -z $BV_HIT ]] || return 0
  fi
  case $words${BV_SEG_DOC[$1]} in
    *[Dd][Rr][Oo][Pp]*|*[Tt][Rr][Uu][Nn][Cc][Aa][Tt][Ee]*|*[Dd][Ee][Ll][Ee][Tt][Ee]*) bv_sql_check ;;
  esac
}

# bv_validate COMMAND: BV_HIT is the first matching rule category, or empty.
bv_validate() {
  local i=0
  export LC_ALL=C
  BV_HIT=''
  BV_SEG_WORDS=(); BV_SEG_REDIR=(); BV_SEG_DOC=(); BV_SEG_PROG=()
  BV_SEG_DEPTH=(); BV_SEG_PARENT=()
  [[ -n ${BV_ARGV_PROGS-} ]] || bv_index_rules
  bv_run_parse "$1" 0 -1
  while (( i < ${#BV_SEG_WORDS[@]} )); do
    bv_check_seg "$i"
    [[ -z $BV_HIT ]] || return 0
    i=$(( i + 1 ))
  done
  return 0
}
# <<< bash-validator generic section <<<

# This edition assumes no framework, so schema-destroying subcommands are
# blocked when a PHP console runner invokes them: a known runner (artisan,
# console, phinx, doctrine-migrations, the Doctrine ORM CLI doctrine) or,
# for a namespaced command name, any `php <script>`. The same tokens in
# documentation and read-only searches are not invocations. Doctrine
# Migrations resolves 0 like first and current-N like prev; Phinx migrates
# down to a -t/--target of 0. Options whose value is the next word come
# from Symfony Console (--env), Doctrine Migrations and Phinx (-t is left
# out on purpose: its value 0 is then the positional `migrate 0`).
BV_CONSOLE_ENTRIES='artisan,console,phinx,doctrine-migrations,doctrine'
BV_CONSOLE_PHP_SCRIPTS=1
BV_CONSOLE_VALUES='--env,--environment,--em,--conn,--configuration,--db-configuration,--write-sql,--all-or-nothing,--parser,--date'
BV_CONSOLE_SHORT_VALUES='ecpd'
BV_FRAMEWORK_RULES=(
  "console|migrate:fresh||destructive command - console migrate:fresh"
  "console|migrate:refresh||destructive command - console migrate:refresh"
  "console|migrate:reset||destructive command - console migrate:reset"
  "console|migrate:rollback||destructive command - console migrate:rollback"
  "console|db:wipe||destructive command - console db:wipe"
  "console|orm:schema-tool:drop||destructive command - doctrine orm:schema-tool:drop"
  "console|rollback||destructive command - phinx rollback"
  "console|migrate|has:-t0,-t=0,--target=0|destructive command - phinx migrate -t 0"
  "console|migrate|arg:prev,first,0,current-[1-9]*|destructive command - doctrine-migrations migrate down"
  "console|migrations:migrate|arg:prev,first,0,current-[1-9]*|destructive command - doctrine-migrations migrate down"
  "console|execute|has:--down|destructive command - doctrine-migrations execute --down"
  "console|migrations:execute|has:--down|destructive command - doctrine-migrations execute --down"
)

bv_validate "$COMMAND"
if [ -n "$BV_HIT" ]; then
  echo "BLOCKED: Command refused (rule category: $BV_HIT)." >&2
  echo "   This operation is blocked. See AGENTS.md." >&2
  exit 2
fi

# Repetition guard. The file-edit counterpart lives in loop-detection.sh; a
# command loop is invisible to it, because rerunning one failing command
# forever touches no file. This hook sees the call before it runs and cannot
# see its result, so identical invocations are the only signal available -
# and they are counted per exact command string, so any real change of
# approach starts its own count. A file edit that loop-detection.sh recorded
# since the command last ran starts its count over too: edit-and-rerun is
# progress, and only a rerun with nothing changed is the loop this guards.
# Only a command that passed the rules above is counted: a refused one never
# ran.
#
# Polling is not a loop. A read-only status query standing alone - CI checks
# and runs (gh pr checks/status/view, gh run list/view/watch), git status, a
# container's or a cluster's state and logs (docker [compose] ps/logs,
# kubectl get/describe/logs), a log tail - changes its answer without any
# edit, so asking again is waiting and is not counted. Optionally after a
# `sleep N &&` and piped into filters; chained with anything else it counts
# like any command.
BV_POLL_QUERY='^(sleep [0-9.]+[smhd]? *(&&|;) *)?(git status|gh pr (checks|status|view)|gh run (list|view|watch)|docker (compose )?(ps|logs)|kubectl (get|describe|logs)|tail)( [^;&`$()<>]*)?$'
if [[ "$COMMAND" =~ $BV_POLL_QUERY ]] && [[ "$COMMAND" != *'||'* ]] && [[ "$COMMAND" != *$'\n'* ]]; then
  exit 0
fi

# The window is a session. Counts are keyed by the host's session id (Claude
# Code and Codex send session_id, Cursor conversation_id), so two sessions or
# a Harness run in one checkout never add to each other's counts, and the
# session start hook clears only its own. A payload without one shares the
# key "shared".
SESSION_KEY=$(printf '%s' "$INPUT" \
  | sed -n -E 's/.*"(session_id|conversation_id)"[[:space:]]*:[[:space:]]*"([^"]*)".*/\2/p' | head -1)
SESSION_KEY=${SESSION_KEY//[^A-Za-z0-9]/}
SESSION_KEY=${SESSION_KEY:0:64}
[ -n "$SESSION_KEY" ] || SESSION_KEY=shared

# The counter directory is the per-user one loop-detection.sh uses. Its name
# is predictable, so a directory that is already there is used only when it
# is a real directory this user owns - one another user planted could
# otherwise pre-seed a count that blocks a first run, or hold a counter that
# is a link to a file this hook would then overwrite - and a counter that is a
# symbolic link is never read or written. Anything else turns the guard off
# rather than trusting it.
REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
REPO_KEY=$(printf '%s' "$REPO_ROOT" | cksum | cut -d' ' -f1)
TRACK_BASE=${TMPDIR:-/tmp}
TRACK_DIR="${TRACK_BASE%/}/claude-loop-detection-${EUID:-0}-$REPO_KEY"
mkdir -m 700 "$TRACK_DIR" 2>/dev/null
if [ -L "$TRACK_DIR" ] || [ ! -d "$TRACK_DIR" ] || [ ! -O "$TRACK_DIR" ]; then
  exit 0
fi

if command -v md5sum > /dev/null 2>&1; then
  COMMAND_KEY=$(printf '%s' "$COMMAND" | md5sum | cut -d' ' -f1)
elif command -v md5 > /dev/null 2>&1; then
  COMMAND_KEY=$(printf '%s' "$COMMAND" | md5 -q)
else
  COMMAND_KEY=$(printf '%s' "$COMMAND" | cksum | tr -d ' ')
fi
TRACK_FILE="$TRACK_DIR/cmd-$SESSION_KEY-$COMMAND_KEY"
[ -L "$TRACK_FILE" ] && exit 0

# Only digits are a count, read in base 10: "08" would otherwise abort the
# arithmetic and switch the guard off for that command.
COUNT=0
[ -f "$TRACK_FILE" ] && COUNT=$(cat "$TRACK_FILE" 2>/dev/null)
case "$COUNT" in *[!0-9]*|'') COUNT=0 ;; esac
[ "${#COUNT}" -gt 9 ] && COUNT=0
COUNT=$((10#$COUNT))
if [ "$COUNT" -gt 0 ] && [ -n "$(find "$TRACK_DIR" -maxdepth 1 -type f -name 'edit-*' -newer "$TRACK_FILE" -print -quit 2>/dev/null)" ]; then
  COUNT=0
fi
COUNT=$((COUNT + 1))
echo "$COUNT" > "$TRACK_FILE" 2>/dev/null

if [ "$COUNT" -ge 12 ]; then
  {
    printf 'BLOCKED: this exact command has run %s times this session.\n' "$COUNT"
    printf '   Repeating it again is not a new attempt. Either change the\n'
    printf '   command (narrow it, add the failing case, read the output\n'
    printf '   differently) or escalate to /debugger for a root cause.\n'
  } >&2
  exit 2
elif [ "$COUNT" -ge 6 ]; then
  BV_WARNING="WARNING: this exact command has run $COUNT times this session. If it keeps failing, /debugger instead of another rerun."
  # Claude Code and Codex add a PreToolUse hook's additionalContext to the
  # model's context next to the tool result, without blocking the call or
  # touching its permission decision. A warning on stderr with a non-blocking
  # exit code reaches only the user's transcript: the agent would meet the
  # guard first at the block.
  printf '{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"%s"}}\n' "$BV_WARNING"
fi

exit 0
