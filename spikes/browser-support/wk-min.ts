// The smallest thing the frozen WebKit build of Playwright for macOS 14 is asked to do.
import { webkit } from "../../interop/node_modules/playwright-core";
const step = (what: string) => console.log(`${(performance.now() / 1000).toFixed(1)}s ${what}`);
const kill = setTimeout(() => { step("gave up"); process.exit(2); }, 40_000);
const browser = await webkit.launch({ timeout: 20_000 });
step(`launched ${browser.version()}`);
const page = await browser.newPage();
step("page");
page.setDefaultTimeout(10_000);
const found = await page.evaluate(() => ({
	userAgent: navigator.userAgent,
	secure: isSecureContext,
	WebTransport: typeof WebTransport,
	VideoEncoder: typeof VideoEncoder,
	AudioEncoder: typeof AudioEncoder,
}));
step(JSON.stringify(found));
await browser.close();
clearTimeout(kill);
