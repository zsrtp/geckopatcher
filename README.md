# Gecko Patcher

This project is a rebuild of the [romhack-compiler](https://github.com/zsrtp/romhack-compiler) project. The previous project has accumulated issues which made it hard to maintain. This project aims to remedy this situation.

## Summary

This project contains a few executables and libraries:

- __`geckolib`__: The main library containing the business code of the project (file loading/saving, encryption/decryption, patching).
- __`gui/native`__: The ui application which is the counterpart to the commandline `romhack` that runs natively on the user's computer.
- __`gui/web`__: The ui application that runs on a web browser.
- __`romhack`__: The commandline application which is used to patch game backups, and create new patch files.

The patcher is the only application which is meant to create new patches. The GUI interfaces are meant only as a way to use existing patch files to patch game backups.