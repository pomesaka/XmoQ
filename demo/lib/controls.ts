// The bar of controls under the picture of a page of the demo (`#controls`),
// as a call has one: buttons that turn something on and off at once, lists
// of the cameras and microphones, and lists of the page's flags.
//
// What is changed at once and what makes the page load again is each
// page's to say, by which of these it puts in the bar: a flag that decides
// the catalog or an encoder (the size of the picture, its frames a second,
// whether there is sound) cannot change while the tracks go on, so its list
// writes the flag into the address and loads the page again. The address
// stays the one place of the flags: a page opened from it, as the interop
// pairings open them, has what it says.

/** The bar of the page; where a page has none, a bar that is shown nowhere. */
export function controlBar(): HTMLElement {
	return document.getElementById("controls") ?? document.createElement("p");
}

/** Puts `element` at the end of the bar, a space after what is before it. */
function add(bar: HTMLElement, element: HTMLElement): void {
	if (bar.childElementCount > 0) bar.append(" ");
	bar.append(element);
}

/**
 * A button that turns something on and off, with `labels[0]` while it is
 * on and `labels[1]` while it is off (`aria-pressed` while it is off).
 * `turn` resolves with the state it is in after; the button is disabled
 * while it waits.
 */
export function toggleButton(
	bar: HTMLElement,
	options: {
		id: string;
		labels: [string, string];
		on: boolean;
		turn(on: boolean): Promise<boolean>;
	},
): void {
	const button = document.createElement("button");
	button.id = options.id;
	let on = options.on;
	const label = (): void => {
		button.textContent = on ? options.labels[0] : options.labels[1];
		button.setAttribute("aria-pressed", String(!on));
	};
	button.addEventListener("click", () => {
		button.disabled = true;
		options.turn(!on).then(
			(now) => {
				on = now;
				label();
				button.disabled = false;
			},
			() => {
				button.disabled = false;
			},
		);
	});
	label();
	add(bar, button);
}

/**
 * A list of the values of the flag `name` of the address, `fallback` where
 * it is left out. Choosing one writes it there (and takes the flag out for
 * `fallback`) and loads the page again.
 */
export function flagList(
	bar: HTMLElement,
	options: {
		name: string;
		values: { value: string; label: string }[];
		fallback: string;
	},
): void {
	const list = document.createElement("select");
	list.id = `flag-${options.name}`;
	const query = new URLSearchParams(location.search);
	const now = query.get(options.name) ?? options.fallback;
	for (const { value, label } of options.values) {
		list.append(new Option(label, value, value === now, value === now));
	}
	list.addEventListener("change", () => {
		if (list.value === options.fallback) query.delete(options.name);
		else query.set(options.name, list.value);
		location.search = query.toString();
	});
	add(bar, list);
}

/**
 * A list of the cameras (`videoinput`) or microphones (`audioinput`) of the
 * browser. Choosing one calls `choose` with its id; the list says the one
 * in use (`chosen`) again after. The names of the devices are told only
 * after the page was let use one of them, so the list is read again where
 * a device opens (`refresh`) and where they change.
 */
export function deviceList(
	bar: HTMLElement,
	options: {
		id: string;
		kind: "videoinput" | "audioinput";
		chosen(): string | undefined;
		choose(deviceId: string): Promise<void>;
	},
): { refresh(): void } {
	const list = document.createElement("select");
	list.id = options.id;
	const refresh = (): void => {
		void navigator.mediaDevices.enumerateDevices().then((devices) => {
			const ofKind = devices.filter((device) => device.kind === options.kind);
			const chosen = options.chosen();
			list.replaceChildren(
				...ofKind.map(
					(device, index) =>
						new Option(
							device.label === ""
								? `${options.kind} ${index + 1}`
								: device.label,
							device.deviceId,
							false,
							device.deviceId === chosen,
						),
				),
			);
		});
	};
	list.addEventListener("change", () => {
		list.disabled = true;
		options.choose(list.value).finally(() => {
			list.disabled = false;
			refresh();
		});
	});
	navigator.mediaDevices.addEventListener("devicechange", refresh);
	refresh();
	add(bar, list);
	return { refresh };
}
