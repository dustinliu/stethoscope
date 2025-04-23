# Stethoscope
web service monitoring service

## Directory Structure

- `/pulse` - Main monitoring service built with Rust
  - Application crate
  - Provides URL monitoring functionality
  - Uses async/await for concurrent processing
  - Implements worker pool pattern
  - Contains agent, dispatcher, reporter, and worker components

- `/pulse_proc_macros` - Procedural macros used by the pulse service
  - Library crate

- `/web` - Main web service built with Actix-web framework
  - Application crate
  - Provides URL monitoring functionality
  - Provides HTTP API endpoints
  - Provides user interface

