# Stethoscope
web site monitoring service

## Directory Structure

- `/pulse` - Main monitoring service built with Rust
  - It is an application crate
  - Provides URL monitoring functionality
  - Uses async/await for concurrent processing
  - Implements worker pool pattern
  - Contains agent, dispatcher, reporter, and worker components

- `/shared` - Shared code between services
  - It is a library crate
  - Common data structures
  - Shared utilities
  - Type definitions

- `/web` - Main web service built with Actix-web framework
  - It is an application crate
  - Provides URL monitoring functionality
  - Provides HTTP API endpoints
  - Provides user interface

