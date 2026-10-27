#!/usr/bin/env bash
# A demo of its own with the reader of an agent in its room and the publishing
# page open in a headless Chromium, all in the background; then stops them.
#
#   mise run agent-demo -- start --out DIR [--relay-port 33443] [--port 33080]
#       [--rooms-port 33787] [--name claude] [--voice FILE.wav | --tone]
#       [--transcribe-language auto] [--meeting N]
#   mise run agent-demo -- stop --out DIR
#
# `--meeting N` (1 to 8): N participants of the meeting of the room "demo"
# (`/meet`) in place of the publishing page, each saying the voice 2 s after
# the one before, and the reader in the meeting's room (`/meetings/demo`).
# The ports default apart from `mise run demo`'s (4443, 8080, 8787). `start`
# writes each process it starts into DIR/pids (its PID, when it began and its
# command, as `ps` tells them); `stop` ends those, and only those that are
# still the same process, and waits for them and for what they had started
# (the relay, the rooms, the gatherer, the transcriber, Chromium).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

# How long a process gets to end after SIGTERM, with what it started.
END_WAIT_S=15

usage() {
	sed -n '2,16p' "${BASH_SOURCE[0]}" >&2
	exit 2
}

[[ $# -ge 1 ]] || usage
command=$1
shift
out=""
relay_port=33443
port=33080
rooms_port=33787
name=claude
voice="${REPO_ROOT}/fixtures/speech/hello-from-the-relay.wav"
tone=false
transcribe_language=""
meeting=0
while [[ $# -gt 0 ]]; do
	case $1 in
	--out) out=$2 ;;
	--meeting) meeting=$2 ;;
	--relay-port) relay_port=$2 ;;
	--port) port=$2 ;;
	--rooms-port) rooms_port=$2 ;;
	--name) name=$2 ;;
	--voice) voice=$2 ;;
	--transcribe-language) transcribe_language=$2 ;;
	--tone)
		tone=true
		shift
		continue
		;;
	*) usage ;;
	esac
	shift 2
done
[[ -n ${out} ]] || usage
# WHY 8 at most: the meeting receives the video of the first 8 who came
# (@xmoq/meeting), and each page is a Chromium of its own.
[[ ${meeting} =~ ^[0-8]$ ]] || usage
[[ ${meeting} -eq 0 || ${tone} == false ]] || usage
mkdir -p "${out}"
out="$(cd "${out}" && pwd)"
pids="${out}/pids"

# What `ps` tells of a process; empty where there is none.
began_of() { ps -o lstart= -p "$1" 2>/dev/null || true; }
command_of() { ps -ww -o command= -p "$1" 2>/dev/null || true; }

# The processes started by $1, and by those, as "PID<TAB>began<TAB>command".
descendants() {
	local child
	for child in $(pgrep -P "$1" || true); do
		printf '%s\t%s\t%s\n' "${child}" "$(began_of "${child}")" "$(command_of "${child}")"
		descendants "${child}"
	done
}

# Whether the line "PID<TAB>began<TAB>command" is a process that is still
# there and still the same one: a PID of an ended process can be another's.
still_there() {
	local pid began command
	IFS=$'\t' read -r pid began command <<<"$1"
	[[ -n $(began_of "${pid}") ]] &&
		[[ $(began_of "${pid}") == "${began}" ]] &&
		[[ $(command_of "${pid}") == "${command}" ]]
}

# Ends the processes of DIR/pids, the last started first. Fails where one of
# them, or of what they started, is still there after END_WAIT_S.
stop_all() {
	[[ -f ${pids} ]] || {
		echo "nothing to stop: no ${pids}" >&2
		return 0
	}
	local lines=() line pid began command waiting left=0
	while IFS= read -r line; do lines+=("${line}"); done <"${pids}"
	local index
	for ((index = ${#lines[@]} - 1; index >= 0; index--)); do
		line=${lines[index]}
		IFS=$'\t' read -r pid began command <<<"${line}"
		if [[ ! ${pid} =~ ^[1-9][0-9]*$ ]]; then
			echo "left alone: not a PID: ${pid}" >&2
			continue
		fi
		if ! still_there "${line}"; then
			echo "already gone (or its PID is another's now): ${pid} ${command}"
			continue
		fi
		waiting=("${line}")
		while IFS= read -r child; do
			[[ -n ${child} ]] && waiting+=("${child}")
		done < <(descendants "${pid}")
		kill -TERM "${pid}"
		local deadline=$((SECONDS + END_WAIT_S)) still=()
		while :; do
			still=()
			for line in "${waiting[@]}"; do
				still_there "${line}" && still+=("${line}")
			done
			[[ ${#still[@]} -eq 0 || ${SECONDS} -ge ${deadline} ]] && break
			sleep 0.2
		done
		echo "stopped: ${pid} ${command}"
		for line in ${still[@]+"${still[@]}"}; do
			IFS=$'\t' read -r pid began command <<<"${line}"
			echo "still there after ${END_WAIT_S} s: ${pid} ${command}" >&2
			left=1
		done
	done
	if [[ ${left} -eq 0 ]]; then
		rm "${pids}"
	else
		echo "${pids} is kept: run stop again, or end those by their PID" >&2
		return 1
	fi
}

fail() {
	echo "agent-demo: $1" >&2
	stop_all || true
	exit 1
}

# Writes $1 into DIR/pids once its command has $2 in it: a process started
# with a redirection to the pipe is the shell until the pipe opens.
record() {
	local pid=$1 expected=$2 deadline=$((SECONDS + 10))
	until [[ $(command_of "${pid}") == *"${expected}"* ]]; do
		[[ -n $(began_of "${pid}") ]] || fail "${expected} ended as it began"
		[[ ${SECONDS} -lt ${deadline} ]] || fail "${expected} did not begin"
		sleep 0.1
	done
	printf '%s\t%s\t%s\n' "${pid}" "$(began_of "${pid}")" "$(command_of "${pid}")" >>"${pids}"
}

# Waits until the file $2 has a line matching $3 after its first $1 lines, for
# $4 s at the most, while the process $5 is there.
wait_for_line() {
	local skip=$1 file=$2 pattern=$3 deadline=$((SECONDS + $4)) pid=$5
	# WHY not `tail | grep -q`: grep ends at the first match, tail gets
	# SIGPIPE, and pipefail makes the match a failure.
	until [[ -f ${file} ]] && grep -q -- "${pattern}" < <(tail -n "+$((skip + 1))" "${file}"); do
		[[ -n $(began_of "${pid}") ]] || fail "it ended before ${file} said ${pattern}; see ${file}"
		[[ ${SECONDS} -lt ${deadline} ]] || fail "${file} did not say ${pattern} in $4 s"
		sleep 0.5
	done
}

# Waits until something listens on 127.0.0.1:$1, for $2 s at the most, while
# the process $3 is there.
wait_for_port() {
	local port=$1 deadline=$((SECONDS + $2)) pid=$3
	until (exec 3<>"/dev/tcp/127.0.0.1/${port}") 2>/dev/null; do
		[[ -n $(began_of "${pid}") ]] || fail "it ended before 127.0.0.1:${port} listened"
		[[ ${SECONDS} -lt ${deadline} ]] || fail "nothing listened on 127.0.0.1:${port} in $2 s"
		sleep 0.2
	done
}

start() {
	if [[ -e ${pids} ]]; then
		echo "agent-demo: ${pids} is there: a start before was not stopped (stop --out ${out})" >&2
		exit 1
	fi
	cargo build --locked -p xmoq-relay -p xmoq-chat
	cargo build --release --locked -p xmoq-transcriber -p xmoq-agent
	bun install --frozen-lockfile
	local say="${out}/say"
	if [[ ! -p ${say} ]]; then
		[[ ! -e ${say} ]] || {
			echo "agent-demo: ${say} is there and is not a pipe" >&2
			exit 1
		}
		mkfifo "${say}"
	fi
	: >"${pids}"

	local language=()
	[[ -z ${transcribe_language} ]] || language=(--transcribe-language "${transcribe_language}")
	# WHY nohup: what is started here outlives this script, and should not end
	# with the shell or the terminal that ran it.
	nohup bun demo/serve.ts --relay-port "${relay_port}" --port "${port}" \
		--rooms-port "${rooms_port}" --chat-store "${out}/chat.sqlite3" ${language[@]+"${language[@]}"} \
		</dev/null >"${out}/demo.log" 2>&1 &
	local demo=$!
	record "${demo}" "demo/serve.ts"
	wait_for_line 0 "${out}/demo.log" "^publish:" 120 "${demo}"
	# WHY: the demo says where the rooms are when it has started them, not
	# when workerd listens; the reader asks again for a while, but a reader
	# begun on a room that is there says nothing of the refusals.
	wait_for_port "${rooms_port}" 60 "${demo}"

	# WHY held open: the reader reads stdin to its end, and the first `echo`
	# into a pipe that nobody else holds would be its end.
	# Its stdin and stderr too: one left to this script's would keep whatever
	# reads this script's output waiting for its end.
	nohup sleep 2147483647 </dev/null >"${say}" 2>/dev/null &
	local holder=$!
	# WHY the log is waited on, and not events/: the reader writes each line
	# on stdout at once, and in a file of events/ only a second later; the
	# log is begun anew with each start.
	local events="${out}/events" agent_log="${out}/agent.log"
	local room="http://127.0.0.1:${rooms_port}/broadcasts/demo"
	[[ ${meeting} -eq 0 ]] || room="http://127.0.0.1:${rooms_port}/meetings/demo"
	nohup target/release/xmoq-agent --room "${room}" \
		--name "${name}" --out "${out}" \
		<"${say}" >"${agent_log}" 2>&1 &
	local agent=$!
	record "${holder}" "sleep"
	record "${agent}" "xmoq-agent"

	if [[ ${meeting} -eq 0 ]]; then
		local sound=(--voice "${voice}")
		[[ ${tone} == false ]] || sound=(--tone)
		nohup bun demo/open-publish.ts --port "${port}" "${sound[@]}" \
			</dev/null >"${out}/page.log" 2>&1 &
		local page=$!
		record "${page}" "demo/open-publish.ts"
		wait_for_line 0 "${out}/page.log" "^opened " 60 "${page}"
		wait_for_line 0 "${agent_log}" '"state":"began","type":"broadcast"' 60 "${agent}"
		grep -E "^(no )?captions:" "${out}/demo.log" || true
		cat <<EOF
the reader is in ${room} as ${name}, and the broadcast began
  say:    echo 'hello' > ${say}      (/frame for a picture)
  read:   ${events}/   (a file a second, in the order of their names)
  logs:   ${out}/demo.log, ${out}/agent.log, ${out}/page.log
  watch:  http://localhost:${port}/watch
  stop:   mise run agent-demo -- stop --out ${out}
EOF
		return
	fi

	# Page i says the voice 2 i s into a round of 2 (N - 1) + 1.5 s of
	# silence and the voice: each begins 2 s after the one before, and the
	# rounds stay as long, so that they stay apart.
	local index ids=() logs=() page
	for ((index = 0; index < meeting; index++)); do
		local log="${out}/meet-$((index + 1)).log"
		nohup bun demo/open-publish.ts --port "${port}" --voice "${voice}" --meet \
			--before-s "$((2 * index))" --after-s "$((2 * (meeting - 1 - index) + 1)).5" \
			</dev/null >"${log}" 2>&1 &
		page=$!
		record "${page}" "demo/open-publish.ts"
		wait_for_line 0 "${log}" "^seated as " 60 "${page}"
		ids+=("$(sed -n 's/^seated as //p' "${log}")")
		logs+=("${log}")
	done
	local id
	for id in "${ids[@]}"; do
		wait_for_line 0 "${agent_log}" "\"id\":\"${id}\",\"state\":\"joined\",\"type\":\"participant\"" 60 "${agent}"
	done
	grep -E "^(no )?captions:" "${out}/demo.log" || true
	cat <<EOF
the reader is in ${room} as ${name}, with ${meeting} participants: ${ids[*]}
  say:    echo 'hello' > ${say}      (said in the room's chat)
  frame:  echo '/frame ${ids[0]}' > ${say}   (a picture of that participant; /frame alone, of the one who spoke last)
  who:    cat ${events}/*.jsonl | grep '"type":"participant"'   (each one's id, as it joins and leaves)
  read:   ${events}/   ("speech" lines, each with the participant's namespace as "from"; a file a second)
  logs:   ${out}/demo.log, ${out}/agent.log, ${logs[*]}
  meet:   http://localhost:${port}/meet   (one more participant, in a browser)
  stop:   mise run agent-demo -- stop --out ${out}
EOF
}

case ${command} in
start) start ;;
stop) stop_all ;;
*) usage ;;
esac
