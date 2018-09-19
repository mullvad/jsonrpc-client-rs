# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/en/1.0.0/)
and this project adheres to [Semantic Versioning](http://semver.org/spec/v2.0.0.html).


## [Unreleased]
### Changed
- Bumped minimum version requirement from 1.26 to 1.28

### Added
- Added subscription support
- Added IPC transport


## [0.5.0] - 2018-06-25
### Changed
- Upgrade error-chain dependency to 0.12 and publicly export it.


## [0.4.0] - 2018-06-18
### Added
- Allow setting custom HTTP headers for RPC requests.
- Allow setting a custom timeout on `HttpTransport`.

### Changed
- Reduce boxing by making the transport future an associated type on the `Transport` trait.
- Disable TLS support by default and rename HttpTransport constructors to something that does not
  change with activation of features.


## [0.3.0] - 2018-03-06
### Changed
- Move logging of json responses from debug to trace level.
- Remove `Clone` bound on `Transport` trait.


## [0.2.2] - 2017-10-05
### Added
- Add integration test in http crate. Testing actual network traffic.

### Fixed
- Fix a bug in the HTTP transport that made a dropped RpcRequest yield an error in the Future
  running on the event loop.


## [0.2.1] - 2017-09-11
### Added
- Add badges to Cargo.toml.

### Changed
- Hide internal macro `expand_params` from documentation.
- Upgrade `error-chain` to 0.11 to get rid of warnings on nightly.
- Use deserialization support on `jsonrpc-core::Error` instead of implementing it manually.

### Fixed
- Fix repository url in Cargo.toml.

### Security
- Upgrade `jsonrpc-core` to 7.1.1 to avoid possible `ErrorKind` deserialization panic.


## [0.2.0] - 2017-08-31
### Changed
- Make transport implementations responsible for IDs for requests.
- Change core Transport trait to return futures and have the error as associated type.
- Rewrite basically everything into an async fashion with futures.


## [0.1.0] - 2017-07-19
### Added
- Initial release with support for synchronous JSON-RPC 2.0 calls.
