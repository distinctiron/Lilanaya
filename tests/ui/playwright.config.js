// Playwright runs inside its own container against the whole site (compose.test.yaml).
// Run with `./test ui` from the repository root; see tests/README.md.
import { defineConfig, devices } from '@playwright/test';

const ci = Boolean(process.env.CI);

export default defineConfig({
	testDir: './specs',
	outputDir: './test-results',
	globalSetup: './global-setup.js',
	// Every test makes its own user, so tests do not get in each other's way.
	fullyParallel: true,
	forbidOnly: ci,
	retries: ci ? 1 : 0,
	workers: ci ? 2 : 4,
	timeout: 45_000,
	expect: {
		timeout: 10_000,
		// Screenshots are compared with a little tolerance: font smoothing differs between machines.
		toHaveScreenshot: { maxDiffPixelRatio: 0.01, animations: 'disabled' }
	},
	reporter: [['list'], ['html', { outputFolder: './report', open: 'never' }]],
	// Screenshots are taken in this container only, so their names say nothing about the machine.
	snapshotPathTemplate: '{testDir}/../screenshots/{projectName}/{arg}{ext}',
	use: {
		baseURL: process.env.BASE_URL ?? 'http://localhost:3100',
		// The site shows every animation finished under "reduce motion", which is what makes the tests steady.
		// The tests of the animations themselves turn motion back on.
		contextOptions: { reducedMotion: 'reduce' },
		trace: 'retain-on-failure',
		screenshot: 'only-on-failure'
	},
	projects: [
		{ name: 'desktop', use: { ...devices['Desktop Chrome'], viewport: { width: 1280, height: 800 } }, testIgnore: /phone\.spec\.js/ },
		// An iPhone, in Safari's engine: the journeys that matter most, at phone width.
		{ name: 'phone', use: { ...devices['iPhone 13'] }, testMatch: /phone\.spec\.js/ }
	]
});
