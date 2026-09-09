# Mock artwork

Ferrari, Formula 1 and The NFL are the unmodified episode images referenced in
`data/tests/transistor_acquired.xml`, downloaded on 2026-09-08. Their publishers
retain ownership. These files illustrate the existing podcast artwork slots.

`acquired-capture.png` is an unchanged copy of the 2026-09-08 physical-device episode-list
capture. The Acquired feed image URL no longer answered, so the mock displays
its artwork region through CSS background positioning. This also keeps the
mock stable when CI replaces the screenshot inventory. Other unavailable art
uses the app's waveform placeholder.
