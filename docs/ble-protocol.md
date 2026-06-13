# BLE Protocol Documentation

This document describes the Bluetooth Low Energy (BLE) protocol implementation used by EmulStick to communicate with host devices.

## Overview

EmulStick uses Web Bluetooth API to connect to a host device and provides:
- Keyboard input emulation
- Mouse movement and click emulation
- Wheel/scroll emulation

## Service and Characteristics UUIDs

| Type | UUID |
|------|------|
| Custom Service | `0000F800-0000-1000-8000-00805f9b34fb` |
| Keyboard Characteristic | `0000F801-0000-1000-8000-00805f9b34fb` |
| Mouse Characteristic | `0000F803-0000-1000-8000-00805f9b34fb` |

## Connection Flow

1. Request and connect to BLE device using Web Bluetooth API
2. Get primary service using the Custom Service UUID
3. Get keyboard characteristic using Keyboard Characteristic UUID
4. Get mouse characteristic using Mouse Characteristic UUID

## Protocol Packets

All packets are 8 bytes (64 bits) in length.

### Keyboard Events

**Key Down / Key Up Packet:**

| Byte | Description |
|------|-------------|
| 0 | Operation flags (modifier keys) |
| 1 | Reserved (0) |
| 2 | Key code |
| 3-7 | Reserved (0) |

**Operation Flags (Byte 0):**
- Bit 0: Left Control
- Bit 1: Left Shift
- Bit 2: Left Alt
- Bit 3: Left GUI (Windows/Command)
- Bit 4: Right Control
- Bit 5: Right Shift
- Bit 6: Right Alt
- Bit 7: Right GUI

### Mouse Events

**Mouse Move / Button Packet:**

| Byte | Description |
|------|-------------|
| 0 | Operation (button state) |
| 1 | X position (low byte, Int16) |
| 2 | X position (high byte, Int16) |
| 3 | Y position (low byte, Int16) |
| 4 | Y position (high byte, Int16) |
| 5 | Wheel value |
| 6-7 | Reserved (0) |

**Operation Flags (Byte 0):**
- Bit 0: Left button
- Bit 1: Right button
- Bit 2: Middle button

**Position Encoding:**
- X and Y positions are signed 16-bit integers (Int16) in little-endian format
- Range: -2047 to +2047

### Wheel Event

| Byte | Description |
|------|-------------|
| 0 | 0x00 |
| 1-4 | Reserved (0) |
| 5 | Wheel value (signed) |
| 6-7 | Reserved (0) |

## Key Code Mapping

USB HID key codes:

| Key | Code | Key | Code | Key | Code |
|-----|------|-----|------|-----|------|
| A | 4 | 1 | 30 | ENTER | 40 |
| B | 5 | 2 | 31 | ESCAPE | 41 |
| C | 6 | 3 | 32 | BACKSPACE | 42 |
| D | 7 | 4 | 33 | TAB | 43 |
| E | 8 | 5 | 34 | SPACE | 44 |
| F | 9 | 6 | 35 | MINUS | 45 |
| G | 10 | 7 | 36 | EQUALS | 46 |
| H | 11 | 8 | 37 | OPEN_BRACKET | 47 |
| I | 12 | 9 | 38 | CLOSE_BRACKET | 48 |
| J | 13 | 0 | 39 | BACK_SLASH | 49 |
| K | 14 | | | SEMICOLON | 51 |
| L | 15 | | | QUOTE | 52 |
| M | 16 | | | BACK_QUOTE | 53 |
| N | 17 | | | COMMA | 54 |
| O | 18 | | | PERIOD | 55 |
| P | 19 | | | SLASH | 56 |
| Q | 20 | | | CAPS_LOCK | 57 |
| R | 21 | | | F1 | 58 |
| S | 22 | | | F2 | 59 |
| T | 23 | | | F3 | 60 |
| U | 24 | | | F4 | 61 |
| V | 25 | | | F5 | 62 |
| W | 26 | | | F6 | 63 |
| X | 27 | | | F7 | 64 |
| Y | 28 | | | F8 | 65 |
| Z | 29 | | | F9 | 66 |