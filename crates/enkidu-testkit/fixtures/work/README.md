# The sample Work index

`sample.ts` makes the sample: invented tickets and PRs of a small shop, shaped like what the core sends
(`enkidu_core::work::model::Snapshot`). The tests of the Work page stand on it.

`sample.json` is what it gives for 8 Nov 2026, 10:00 UTC and the home folder `/Users/demo`. Rust reads the JSON
(`enkidu_testkit::WORK_SAMPLE`); node is not needed to run a test. To make it again, in this folder:

    node --experimental-strip-types --no-warnings -e 'import("./sample.ts").then((m) => { const now = Date.parse("2026-11-08T10:00:00Z"); process.stdout.write(JSON.stringify(m.sampleWork(now, "/Users/demo", "main"), null, 1) + "\n"); })' > sample.json

`rich-doc.json` is one ticket description in Jira's document format with every kind of block the card draws, for
its pictures and screen tests (`enkidu_testkit::WORK_RICH_DOC`).
