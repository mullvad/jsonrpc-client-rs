# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/en/1.0.0/)
and this project adheres to [Semantic Versioning](http://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Changed
- Made internal macro `expand_params` hidden from documentation.

## [0.2.0] - 2017-08-31
### Changed
- Make transport implementations responsible for IDs for requests.
- Change core Transport trait to return futures and have the error as associated type.
- Rewrote basically everything into an async fashion with futures.

## [0.1.0] - 2017-07-19
### Added
- Initial release with support for synchronous JSON-RPC 2.0 calls.
