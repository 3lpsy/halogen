# halogen-e2e

Browser journeys against the real application.

- Build dist with just ui-build; just test-e2e starts the server and ChromeDriver.
- HALOGEN_E2E_REQUIRED makes missing browser or frontend prerequisites fail.
- Upstream feeds are mocked; browser navigation and API calls remain real.
