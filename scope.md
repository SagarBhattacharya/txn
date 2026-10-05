# Project Scope: Double-Entry Financial Ledger Engine (`txn`)

## 1. Project Goal
Build a reliable, production-ready financial ledger service in Rust. The system 
records all money movements accurately, prevents accounts from going into negative balances, 
guarantees that no network retry causes a double payment, ensures every operation is tied 
to an authenticated user, and keeps a permanent audit trail by never deleting transaction history.

---

## 2. Milestone 1: Double-Entry Accounting Core
* Record all transfers using double-entry bookkeeping rules where money leaving one account equals money entering another.
* Require every transaction to involve at least two distinct accounts.
* Prevent asset accounts from dropping below zero by verifying balances before confirming transfers.
* Keep accounting records append-only so past transactions are never altered or deleted.

---

## 3. Milestone 2: Concurrency & Duplicate Request Protection
* Lock affected accounts in a predictable order so concurrent transfers never freeze or deadlock each other.
* Require every transfer and reversal request to include a unique reference token from the client.
* Detect and handle repeated requests safely so automatic network retries return the original receipt instead of charging again.
* Coordinate simultaneous identical requests cleanly at the database level without raising server crashes.

---

## 4. Milestone 3: Safe Transaction Reversals
* Allow cancelling an earlier transaction by writing an equal and opposite transaction rather than editing the database.
* Ensure a transaction cannot be reversed more than once.
* Prevent a reversal transaction itself from being reversed.
* Verify that the account giving money back has sufficient funds before allowing the reversal to proceed.

---

## 5. Milestone 4: User Authentication & Access Control
* Store user credentials securely using industry-standard password hashing.
* Support user registration and login endpoints that issue secure authentication tokens.
* Require valid authentication headers on all account, transfer, and reversal operations.
* Associate created accounts and recorded transactions with the authenticated user to prevent unauthorized access.

---

## 6. Milestone 5: Observability & Production Operations
* Provide health check endpoints for monitoring systems to verify that both the server process and database connection are working.
* Format operational server logs consistently with request tracking IDs to make troubleshooting straightforward.
* Shut down the application cleanly on system stop signals so ongoing transfers finish recording before the service exits.

---

## 7. Milestone 6: Automated Testing & Continuous Integration
* Maintain an automated test suite that checks domain rules, database queries, and API routes in an isolated testing environment.
* Test that concurrent transfers and duplicate requests behave correctly without data corruption.
* Run code formatting, code quality linters, and all tests automatically on every pull request.

---

## 8. Milestone 7: Performance Verification
* Measure how many transfers the engine can handle per second under realistic conditions.
* Measure response delays when multiple clients attempt to transfer money through the same account at the same time.
* Record memory usage and latency figures in the project documentation to show system limits.

---

## 9. Items Out of Scope
* Multiple currencies and foreign exchange conversions (the engine works entirely with a single currency).
* Distributed databases and multi-node transaction coordination (the system relies on a single relational database).
* Third-party social logins or single sign-on providers (the engine relies directly on email/username and password token authentication).