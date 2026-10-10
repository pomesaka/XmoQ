import { createFetchStreamDecoder } from "<repository>/packages/moqt/src/core/fetch.ts";
const lines = (await Bun.file(process.argv[2]!).text()).trim().split("\n");
for (const line of lines) {
	const [name, order, hexText] = line.split(" ");
	const bytes = Uint8Array.from(hexText!.match(/../g)!.map((h) => Number.parseInt(h, 16)));
	const decoder = createFetchStreamDecoder({ maxObjectPayloadLength: 1 << 20, maxObjectPropertiesLength: 1024 }, order === "descending" ? "Descending" : "Ascending");
	const out: string[] = [];
	for (const byte of bytes) {
		const result = decoder.push(Uint8Array.of(byte));
		for (const item of result.items) {
			if (item.type === "header") out.push(`header(${item.header.requestId})`);
			else if (item.type === "end-of-range") out.push(`end(${item.kind},${item.location.group},${item.location.object})`);
			else out.push(`obj(${item.object.groupId},${item.object.objectId},sub=${item.object.subgroupId ?? "none"},prio=${item.object.publisherPriority},len=${item.object.payload.length})`);
		}
		if (result.failure) out.push("FAILURE " + JSON.stringify(result.failure, (_, v) => typeof v === "bigint" ? v.toString() : v));
	}
	const fin = decoder.finish();
	console.log(name, out.join(" "), fin ? "FIN-ERROR " + fin.message : "");
}
