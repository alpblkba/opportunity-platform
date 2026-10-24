# Opportunity platform

A personal application for discovering, filtering and tracking jobs, internships and thesis opportunities.

The application collects postings from supported public sources, stores them in PostgreSQL and provides search, matching and application tracking through a web interface.

## Stack

Rust, Axum, SQLx, PostgreSQL, React, TypeScript and Docker.

## Local setup

Requires Rust, Node.js 22 or newer, and Docker with PostgreSQL 16.

Start the application:

`./run.sh`

The web interface is available at `http://127.0.0.1:8787`.

Stop the application with Ctrl+C. Run `./stop.sh` to stop the local database.

## Current limitations

The application currently supports one owner account. Matching is rule-based, with no semantic ranking. External employer application submissions are disabled by default.
