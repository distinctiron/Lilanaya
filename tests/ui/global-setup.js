// Waits until the site answers before the first test starts: the API may still be connecting to its database.
export default async function globalSetup(config) {
	const base = config.projects[0].use.baseURL;
	const deadline = Date.now() + 120_000;
	let last = 'no answer yet';
	while (Date.now() < deadline) {
		try {
			const res = await fetch(`${base}/api/health`);
			if (res.ok && (await res.json()).status === 'ok') return;
			last = `HTTP ${res.status}`;
		} catch (error) {
			last = error.cause?.code ?? error.message;
		}
		await new Promise((resolve) => setTimeout(resolve, 1000));
	}
	throw new Error(`The site at ${base} did not come up within two minutes (${last}). See \`docker compose -f compose.test.yaml logs api web\`.`);
}
