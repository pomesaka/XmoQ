#!/usr/bin/env bash
# Fetches a model of whisper.cpp for xmoq-speech, and checks its sha256.
#
#   mise run speech-model                      # the default model
#   mise run speech-model -- small-q5_1        # another one of the list below
#   mise run speech-model -- --dir /some/where tiny
#
# The models are not in the repository: the smallest is 74 MiB. They are put in
# ${XDG_CACHE_HOME:-~/.cache}/xmoq/whisper/, where xmoq-speech looks by default
# (`default_model_dir` in crates/speech/src/model.rs), unless --dir says otherwise.
set -euo pipefail

# WHY Hugging Face's ggerganov/whisper.cpp: the repository whose models
# whisper.cpp's own models/download-ggml-model.sh fetches.
SOURCE="https://huggingface.co/ggerganov/whisper.cpp/resolve/main"

# The sha256 of each, as Hugging Face's API listed it (the LFS oid of the
# file in ggerganov/whisper.cpp), read on 2026-10-25. A model that is not
# here is not fetched: a file that changes there is not taken without a look.
# WHY a case and not an associative array: macOS's /bin/bash is 3.2, which
# has none.
sha256_expected() {
	case "$1" in
	tiny) echo be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21 ;;
	small-q5_1) echo ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb ;;
	large-v3-turbo-q5_0) echo 394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2 ;;
	*) echo "" ;;
	esac
}

# Kept the same as `DEFAULT_MODEL` in crates/speech/src/model.rs.
DEFAULT_MODEL=large-v3-turbo-q5_0

dir="${XDG_CACHE_HOME:-${HOME}/.cache}/xmoq/whisper"
name="${DEFAULT_MODEL}"
while (($# > 0)); do
	case "$1" in
	--dir)
		dir="$2"
		shift 2
		;;
	*)
		name="$1"
		shift
		;;
	esac
done

expected="$(sha256_expected "${name}")"
if [[ -z "${expected}" ]]; then
	echo "unknown model: ${name} (known: tiny, small-q5_1, large-v3-turbo-q5_0)" >&2
	exit 1
fi

file="ggml-${name}.bin"
target="${dir}/${file}"

sha256_of() {
	shasum -a 256 "$1" | cut -d ' ' -f 1
}

if [[ -f "${target}" ]] && [[ "$(sha256_of "${target}")" == "${expected}" ]]; then
	echo "${target} is there already"
	exit 0
fi

mkdir -p "${dir}"
partial="${target}.partial"
curl --fail --location --show-error --progress-bar --output "${partial}" "${SOURCE}/${file}"
actual="$(sha256_of "${partial}")"
if [[ "${actual}" != "${expected}" ]]; then
	rm -f "${partial}"
	echo "${file}: sha256 ${actual}, not ${expected}; not kept" >&2
	exit 1
fi
mv "${partial}" "${target}"
echo "${target}"
