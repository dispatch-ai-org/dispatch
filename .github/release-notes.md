## Dispatch 0.4.2 — setup fix

0.4.2 fixes one defect in 0.4.1: `dispatch setup` could not add or revalidate a
resource profile once the state directory had a database, which is true after any
run. Setup stopped with `Resource unchanged: no such table: capacity_authorizations`.
Claude profiles must be revalidated every 24 hours, so every existing Claude user was
affected within a day.

**Cause.** 0.4.1's migration 24 removed the capacity tables, but setup still read
`capacity_authorizations` to choose the next authorization revision. The setup test
ran against a state with no database, so it never reached that path.

**Fix.** The next authorization revision is now above every revision recorded in
`funding_refusals`, the table that holds 0.4.1's sticky refusals. A re-authorized
profile therefore never reuses a refused revision. Setup still opens the database
read-only and changes nothing in it.

**Tests.** A unit test prepares a setup proposal against a database at the current
schema, with a refusal already recorded. The setup terminal journeys now include one
against a state that already has a database, and they check that setup does not
write to it. Both tests fail on 0.4.1 with the reported error.

**Evidence.** The real `dispatch setup` against the migrated schema-24 state from the
0.4.1 release run reached the authorization screen for its Claude profile. It was
cancelled there, and the configuration was left unchanged.

**Upgrading.** No migration; the schema stays at 24. If setup failed under 0.4.1,
run it again.
