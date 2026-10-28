// The garden of the pet, drawn on a canvas: a floor seen from above and in
// front, which narrows into the room, the pet where its state puts it at a
// moment, its shadow, its bubble, and what was thrown or put down for it.
// Nothing here knows MoQ; `pet.ts` hands it the state and the moment.

/** What the drawing needs of a state of `@xmoq/pet`: places are 0 to 1000 across and into the room. */
export interface GardenState {
	from: readonly [number, number];
	to: readonly [number, number];
	/** By the page's clock, already moved from the pet's (`pet.ts`). */
	began: number;
	arrives: number;
	facing: "left" | "right";
	doing: string;
	mood: number;
	bubble: string | null;
}

/** Something for the pet on the floor: a ball in the air or on the floor, or a bowl. */
export interface Thing {
	kind: "ball" | "bowl";
	at: readonly [number, number];
	/** When it was put down or thrown, by the page's clock. */
	since: number;
}

const FLOOR = 1000;
/** How long a ball flies before it lies on the floor, in ms. */
const FLIGHT_MS = 700;

/** The colours of the garden: a terracotta floor at dusk. */
const INK = "#15171b";
const WALL = "#1d2026";
const FLOOR_NEAR = "#d9bf91";
const FLOOR_FAR = "#b49470";
const TILE_LINE = "rgba(80, 52, 30, 0.18)";
const RIM = "#a65a3a";
const RUG = "#5f7a52";
const RUG_EDGE = "#4b6342";
const AMBER = "#f2c14e";

/** Where a place of the floor is on the canvas, and how large things are there (1 at the front). */
function project(
	width: number,
	height: number,
	[x, y]: readonly [number, number],
): { px: number; py: number; scale: number } {
	// The floor's front edge is wide and low, its back edge narrow and high.
	const depth = y / FLOOR;
	const scale = 1 - 0.45 * depth;
	const front = { y: height * 0.9, half: width * 0.44 };
	const back = { y: height * 0.36, half: width * 0.26 };
	const half = front.half + (back.half - front.half) * depth;
	const py = front.y + (back.y - front.y) * depth;
	const px = width / 2 + (x / FLOOR - 0.5) * 2 * half;
	return { px, py, scale };
}

function easeInOut(t: number): number {
	return t < 0.5 ? 2 * t * t : 1 - (-2 * t + 2) ** 2 / 2;
}

/** Where the pet is at `now`, and how far along its way. */
function placeOf(
	state: GardenState,
	now: number,
): { at: [number, number]; along: number } {
	const span = state.arrives - state.began;
	const along =
		span <= 0 ? 1 : Math.min(1, Math.max(0, (now - state.began) / span));
	const eased = easeInOut(along);
	return {
		at: [
			state.from[0] + (state.to[0] - state.from[0]) * eased,
			state.from[1] + (state.to[1] - state.from[1]) * eased,
		],
		along,
	};
}

function drawRoom(
	context: CanvasRenderingContext2D,
	w: number,
	h: number,
): void {
	context.fillStyle = INK;
	context.fillRect(0, 0, w, h);
	// A back wall with a window of evening light.
	const corner = (x: number, y: number) => project(w, h, [x, y]);
	const backLeft = corner(0, FLOOR);
	const backRight = corner(FLOOR, FLOOR);
	context.fillStyle = WALL;
	context.fillRect(
		backLeft.px,
		h * 0.08,
		backRight.px - backLeft.px,
		backLeft.py - h * 0.08,
	);
	const glow = context.createRadialGradient(
		w * 0.62,
		h * 0.2,
		4,
		w * 0.62,
		h * 0.2,
		w * 0.3,
	);
	glow.addColorStop(0, "rgba(242, 193, 78, 0.28)");
	glow.addColorStop(1, "rgba(242, 193, 78, 0)");
	context.fillStyle = glow;
	context.fillRect(
		backLeft.px,
		h * 0.08,
		backRight.px - backLeft.px,
		backLeft.py - h * 0.08,
	);
	context.fillStyle = "rgba(242, 193, 78, 0.55)";
	context.fillRect(w * 0.56, h * 0.13, w * 0.12, h * 0.13);
	context.strokeStyle = WALL;
	context.lineWidth = 3;
	context.beginPath();
	context.moveTo(w * 0.62, h * 0.13);
	context.lineTo(w * 0.62, h * 0.26);
	context.moveTo(w * 0.56, h * 0.195);
	context.lineTo(w * 0.68, h * 0.195);
	context.stroke();

	// The side walls, darker towards the front, so that the box has depth.
	const frontLeft = corner(0, 0);
	const frontRight = corner(FLOOR, 0);
	for (const [far, near] of [
		[backLeft, frontLeft],
		[backRight, frontRight],
	]) {
		if (far === undefined || near === undefined) continue;
		const side = context.createLinearGradient(0, h * 0.08, 0, near.py);
		side.addColorStop(0, "#22262d");
		side.addColorStop(1, "#16181c");
		context.fillStyle = side;
		context.beginPath();
		context.moveTo(far.px, h * 0.08);
		context.lineTo(far.px, far.py);
		context.lineTo(near.px, near.py);
		context.lineTo(near.px, 0);
		context.closePath();
		context.fill();
	}
	// The floor, lighter at the front.
	const floor = context.createLinearGradient(0, backLeft.py, 0, frontLeft.py);
	floor.addColorStop(0, FLOOR_FAR);
	floor.addColorStop(1, FLOOR_NEAR);
	context.fillStyle = floor;
	context.beginPath();
	context.moveTo(backLeft.px, backLeft.py);
	context.lineTo(backRight.px, backRight.py);
	context.lineTo(frontRight.px, frontRight.py);
	context.lineTo(frontLeft.px, frontLeft.py);
	context.closePath();
	context.fill();
	// Tiles: lines across that come closer into the room, and lines into it.
	context.strokeStyle = TILE_LINE;
	context.lineWidth = 1;
	for (let row = 1; row < 8; row++) {
		const y = FLOOR * (1 - (1 - row / 8) ** 1.25);
		const left = corner(0, y);
		const right = corner(FLOOR, y);
		context.beginPath();
		context.moveTo(left.px, left.py);
		context.lineTo(right.px, right.py);
		context.stroke();
	}
	for (let column = 1; column < 8; column++) {
		const x = (FLOOR * column) / 8;
		const near = corner(x, 0);
		const far = corner(x, FLOOR);
		context.beginPath();
		context.moveTo(near.px, near.py);
		context.lineTo(far.px, far.py);
		context.stroke();
	}
	// The window's light, fallen on the floor towards the front.
	const light = [
		corner(560, 1000),
		corner(700, 1000),
		corner(640, 380),
		corner(420, 380),
	];
	context.fillStyle = "rgba(255, 214, 140, 0.16)";
	context.beginPath();
	light.forEach((point, index) => {
		if (index === 0) context.moveTo(point.px, point.py);
		else context.lineTo(point.px, point.py);
	});
	context.closePath();
	context.fill();
	// A rug, where the pet likes to sleep.
	const rug = [
		corner(120, 520),
		corner(420, 520),
		corner(420, 820),
		corner(120, 820),
	];
	context.fillStyle = RUG;
	context.strokeStyle = RUG_EDGE;
	context.lineWidth = 3;
	context.beginPath();
	rug.forEach((point, index) => {
		if (index === 0) context.moveTo(point.px, point.py);
		else context.lineTo(point.px, point.py);
	});
	context.closePath();
	context.fill();
	context.stroke();
	// The rim of the box, in front.
	context.fillStyle = RIM;
	context.beginPath();
	context.moveTo(frontLeft.px, frontLeft.py);
	context.lineTo(frontRight.px, frontRight.py);
	context.lineTo(frontRight.px, frontRight.py + h * 0.05);
	context.lineTo(frontLeft.px, frontLeft.py + h * 0.05);
	context.closePath();
	context.fill();
	// A plant in the far corner.
	const pot = corner(930, 940);
	drawEmoji(context, "🪴", pot.px, pot.py, 64 * pot.scale);
}

/** The corners darker, as a lamp lights the middle of a box. */
function drawVignette(
	context: CanvasRenderingContext2D,
	w: number,
	h: number,
): void {
	const shade = context.createRadialGradient(
		w / 2,
		h * 0.6,
		h * 0.35,
		w / 2,
		h * 0.6,
		w * 0.75,
	);
	shade.addColorStop(0, "rgba(0, 0, 0, 0)");
	shade.addColorStop(1, "rgba(0, 0, 0, 0.45)");
	context.fillStyle = shade;
	context.fillRect(0, 0, w, h);
}

function drawEmoji(
	context: CanvasRenderingContext2D,
	emoji: string,
	x: number,
	y: number,
	size: number,
	flip = false,
): void {
	context.save();
	context.translate(x, y);
	if (flip) context.scale(-1, 1);
	context.font = `${Math.round(size)}px serif`;
	context.textAlign = "center";
	context.textBaseline = "bottom";
	context.fillText(emoji, 0, 0);
	context.restore();
}

function drawShadow(
	context: CanvasRenderingContext2D,
	x: number,
	y: number,
	size: number,
	lift: number,
): void {
	// Smaller and fainter the higher the pet is.
	const shrink = 1 / (1 + lift / 60);
	context.fillStyle = `rgba(40, 24, 12, ${0.32 * shrink})`;
	context.beginPath();
	context.ellipse(
		x,
		y - 2,
		size * 0.42 * shrink,
		size * 0.12 * shrink,
		0,
		0,
		Math.PI * 2,
	);
	context.fill();
}

function drawBubble(
	context: CanvasRenderingContext2D,
	text: string,
	x: number,
	y: number,
): void {
	context.save();
	context.font = "16px ui-monospace, Menlo, monospace";
	const width = context.measureText(text).width + 20;
	const height = 30;
	const left = x - width / 2;
	const top = y - height - 12;
	context.fillStyle = "#fbf4e6";
	context.strokeStyle = "#3b2a1f";
	context.lineWidth = 2;
	context.beginPath();
	context.roundRect(left, top, width, height, 12);
	context.moveTo(x - 7, top + height);
	context.lineTo(x, top + height + 10);
	context.lineTo(x + 7, top + height);
	context.fill();
	context.stroke();
	// Cover the line between the bubble and its tail.
	context.fillRect(x - 6, top + height - 2, 12, 3);
	context.fillStyle = "#3b2a1f";
	context.textAlign = "center";
	context.textBaseline = "middle";
	context.fillText(text, x, top + height / 2 + 1);
	context.restore();
}

/** The pet. What it does is in how it moves, not in another picture. */
const PET = "🐕";

/** The pips of the mood, at the corner: as many lit as the mood, of 5. */
function drawMood(
	context: CanvasRenderingContext2D,
	w: number,
	mood: number,
): void {
	for (let pip = 0; pip <= 4; pip++) {
		context.fillStyle = pip <= mood ? AMBER : "rgba(242, 193, 78, 0.18)";
		context.beginPath();
		context.arc(w - 22 - (4 - pip) * 16, 22, 5, 0, Math.PI * 2);
		context.fill();
	}
}

/** Draws the garden at `now` (the page's clock). */
export function drawGarden(
	context: CanvasRenderingContext2D,
	state: GardenState | undefined,
	things: readonly Thing[],
	now: number,
): void {
	const { width: w, height: h } = context.canvas;
	drawRoom(context, w, h);
	// What is far is drawn first.
	const drawn: { y: number; draw(): void }[] = [];
	for (const thing of things) {
		const place = project(w, h, thing.at);
		const age = now - thing.since;
		if (thing.kind === "bowl") {
			drawn.push({
				y: thing.at[1],
				// A little in front of the pet that eats from it, so that it shows.
				draw: () =>
					drawEmoji(
						context,
						"🥣",
						place.px,
						place.py + 18 * place.scale,
						44 * place.scale,
					),
			});
			continue;
		}
		// A ball flies in from the front, and then lies there.
		const flight = Math.min(1, age / FLIGHT_MS);
		const start = project(w, h, [thing.at[0], 0]);
		const px = start.px + (place.px - start.px) * flight;
		const py = start.py + h * 0.1 + (place.py - start.py - h * 0.1) * flight;
		const lift = Math.sin(Math.PI * flight) * h * 0.25;
		drawn.push({
			y: thing.at[1],
			draw: () => {
				drawShadow(context, px, py, 30 * place.scale, lift);
				drawEmoji(context, "🎾", px, py - lift, 30 * place.scale);
			},
		});
	}
	if (state !== undefined) {
		const { at, along } = placeOf(state, now);
		const place = project(w, h, at);
		const size = 112 * place.scale;
		const moving =
			along < 1 && state.from.some((value, index) => value !== state.to[index]);
		// A step is a little hop; a jump is a big one; a sleeping pet breathes.
		const lift =
			state.doing === "jump"
				? Math.abs(Math.sin(now / 160)) * 48 * place.scale
				: moving
					? Math.abs(Math.sin(now / 90)) * 9 * place.scale
					: 0;
		const breathe =
			state.doing === "sleep" ? 1 + Math.sin(now / 600) * 0.03 : 1;
		drawn.push({
			y: at[1],
			draw: () => {
				drawShadow(context, place.px, place.py, size, lift);
				context.save();
				context.translate(place.px, place.py - lift);
				context.scale(breathe, 1 / breathe);
				if (state.doing === "sleep")
					context.rotate(state.facing === "left" ? -0.25 : 0.25);
				drawEmoji(context, PET, 0, 0, size, state.facing === "right");
				context.restore();
				if (state.doing === "sleep") {
					drawEmoji(
						context,
						"💤",
						place.px + size * 0.45,
						place.py - size * 0.8,
						size * 0.38,
					);
				}
				if (state.bubble !== null) {
					drawBubble(context, state.bubble, place.px, place.py - size - lift);
				}
			},
		});
	}
	// Far into the room first: a greater `y` is further.
	drawn.sort((a, b) => b.y - a.y);
	for (const each of drawn) each.draw();
	drawVignette(context, w, h);
	if (state !== undefined) drawMood(context, w, state.mood);
}

/** The place of the floor under a point of the canvas, or undefined off the floor. */
export function floorAt(
	width: number,
	height: number,
	px: number,
	py: number,
): [number, number] | undefined {
	const front = project(width, height, [0, 0]);
	const back = project(width, height, [0, FLOOR]);
	if (py > front.py || py < back.py) return undefined;
	const depth = (py - front.py) / (back.py - front.py);
	const y = Math.round(depth * FLOOR);
	const left = project(width, height, [0, y]);
	const right = project(width, height, [FLOOR, y]);
	const x = Math.round(((px - left.px) / (right.px - left.px)) * FLOOR);
	if (x < 0 || x > FLOOR) return undefined;
	return [x, y];
}
