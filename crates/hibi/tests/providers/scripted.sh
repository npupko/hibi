#!/bin/sh
# A scripted provider for the CommandProvider tests. POSIX sh only.
#
# Usage: sh scripted.sh <script> <dir>
#
# Writes its process id to <dir>/pid and appends "start" and then every
# request line it reads to <dir>/log. For the n-th request line it runs
# line n of <script>:
#
#   {...}             print the line with every @ID@ replaced by n
#   noise <k> {...}   write <k> KiB to standard error, then print as above
#   sleep             never answer (the process becomes `sleep`)
#   big               print one line of 17 MiB
#   garbage           print a line that is not JSON
#   exit              exit without answering
#
# It exits 0 at end of input or when the script has no more lines.

script=$1
dir=$2
echo $$ > "$dir/pid"
echo start >> "$dir/log"
exec 3< "$script"
n=0
while IFS= read -r request; do
    n=$((n + 1))
    printf '%s\n' "$request" >> "$dir/log"
    IFS= read -r action <&3 || exit 0
    case $action in
        noise\ *)
            rest=${action#noise }
            kib=${rest%% *}
            action=${rest#* }
            dd if=/dev/zero bs=1024 count="$kib" 2>/dev/null | tr '\000' 'e' >&2
            ;;
    esac
    case $action in
        sleep) exec sleep 60 ;;
        big) dd if=/dev/zero bs=1048576 count=17 2>/dev/null | tr '\000' 'a'; echo ;;
        garbage) echo 'this is not json' ;;
        exit) exit 3 ;;
        *) printf '%s\n' "$action" | sed "s/@ID@/$n/g" ;;
    esac
done
exit 0
