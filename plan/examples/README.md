# Example fixtures

reference-project.json is a valid portable v2 project with two independent goals, six epics, eight tasks, scoped branches/joins and mixed review policies. It is a starting snapshot, not evidence that the application is implemented. operations.json supplies schema-valid request/response shapes for every REST operation; those independent examples are not one sequential execution and illustrative IDs must be replaced with real responses. The runnable workflow.py script captures live IDs/revisions and can be run only after v1 implementation.

No real credentials are stored here. The fixture actors are historical labels on import; register fresh agents for live work. Derived eligibility in portable export is recomputed by importer. The workflow descriptions specify exact states/errors that acceptance tests must assert.
