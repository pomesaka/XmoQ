#!/usr/bin/env bash
# Downloads the prebuilt moq-relay release binary into ./bin and checks its SHA-256.
# WHY a prebuilt binary: moq-relay 0.17.2 declares rust-version 1.95, and building it
# would require a newer toolchain than the 1.92 this spike was run with.
set -euo pipefail

VERSION="0.17.2"
case "$(uname -s)-$(uname -m)" in
	Darwin-arm64)
		ASSET="moq-relay-v${VERSION}-aarch64-apple-darwin.tar.gz"
		SHA256="bace64f75d89c8753c54af0413fc5199197b80278c855c8b234d97c1a8cca4ca"
		;;
	Linux-x86_64)
		ASSET="moq-relay-v${VERSION}-x86_64-unknown-linux-gnu.tar.gz"
		SHA256="fc35dda1e45d3f0439f54259467491d079dd409759282660d4590e66044a66e4"
		;;
	Linux-aarch64)
		ASSET="moq-relay-v${VERSION}-aarch64-unknown-linux-gnu.tar.gz"
		SHA256="511b55d35243ee59cf78a25c4d8e78b7754a7b02f66194ee927e0e58aff6f5e0"
		;;
	*)
		echo "no prebuilt moq-relay ${VERSION} for $(uname -s)-$(uname -m)" >&2
		exit 1
		;;
esac

cd "$(dirname "$0")/.."
mkdir -p bin
curl -fsSL -o "bin/${ASSET}" \
	"https://github.com/moq-dev/moq/releases/download/moq-relay-v${VERSION}/${ASSET}"
echo "${SHA256}  bin/${ASSET}" | shasum -a 256 -c -
tar -xzf "bin/${ASSET}" -C bin
find bin -type f -name moq-relay -exec ls -l {} \;
