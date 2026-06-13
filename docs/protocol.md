Emulstick Emulator Keyboard and Mouse 

**Bluetooth Data Format** with 2.4-GHz Bluetooth low energy Systems (Bluetooth LE) 

* 
**Emulator Device**: emulstick native combo keyboard and mouse 


* 
**Compliance**: Bluetooth v4.0 Compliant 


* **Hardware Variations**:
* (USB vid:0x0451 pid:0xe010; Dongle Firmware 1.2.x App download and update version) 


* (USB vid:0x4348 pid:0xe010; Dongle Firmware 1.3.0) 





---

I. Bluetooth Services 

1. Device Information Service GATT SERVICE UUID: `0000180A-0000-1000-8000-00805f9b34fb` (General) General attributes, used to read device information.

| CLIENT CHARACTERISTIC UUID | Content | Type | Write | Read | Notify |
| --- | --- | --- | --- | --- | --- |
| `00002A23-0000-1000-8000-00805f9b34fb` | System ID | 8 Bytes | No | Yes | No |
| `00002A24-0000-1000-8000-00805f9b34fb` | Model Number | String | No | Yes | No |
| `00002A25-0000-1000-8000-00805f9b34fb` | Serial Number | String | No | Yes | No |
| `00002A26-0000-1000-8000-00805f9b34fb` | Firmware Revision | String | No | Yes | No |
| `00002A27-0000-1000-8000-00805f9b34fb` | Hardware Revision | String | No | Yes | No |
| `00002A28-0000-1000-8000-00805f9b34fb` | Software Revision | String | No | Yes | No |
| `00002A29-0000-1000-8000-00805f9b34fb` | Manufacturer Name | String | No | Yes | No |
| `00002A50-0000-1000-8000-00805f9b34fb` | Product ID | 7 Bytes | No | Yes | No |
| <br>(Table Data Source: )

 |  |  |  |  |  |

*Note: Among the above data, items 2A23 and 2A26 have practical utility*.

2. Custom Service GATT SERVICE UUID: `0000F800-0000-1000-8000-00805f9b34fb` 5 attributes, serving as 5 data transmission channels.

| CLIENT CHARACTERISTIC UUID | Emulator Sub-device | Write | Read | Notify |
| --- | --- | --- | --- | --- |
| `0000F801-0000-1000-8000-00805f9b34fb` | Keyboard | Yes (without response) | Yes | Yes |
| `0000F802-0000-1000-8000-00805f9b34fb` | Gamepad | Yes (without response) | No | Ignore |
| `0000F803-0000-1000-8000-00805f9b34fb` | Mouse | Yes (without response) | No | Ignore |
| `0000F804-0000-1000-8000-00805f9b34fb` | Pen & Consumer (Stylus and Control Keys) | Yes (without response) | No | Ignore |
| `0000F80F-0000-1000-8000-00805f9b34fb` | Private Control | Yes (without response) | No | Yes |
| <br>(Table Data Source: )

 |  |  |  |  |

---

### II. 

CHARACTERISTIC UUID: F801 

**Write: HID Keyboard User Report (complies with general format specifications)** 

* 
**Data length**: 8 Bytes 


* 
**Byte0**: Control keys (1 is valid, 0 is invalid) 


* bit7: RightGui 


* bit6: RightAlt 


* bit5: RightShift 


* bit4: RightCtrl 


* bit3: LeftGui 


* bit2: LeftAlt 


* bit1: LeftShift 


* bit0: LeftCtrl 




* 
**Byte1**: Fixed as `0x00` 


* 
**Byte2-Byte7**: keyboard button usage id (Min:4, Max:106), 0 when invalid. *(Note: See Appendix for Keyboard Usage ID Table)*.



**Read or Notify: HID Keyboard Led Report (complies with general format specifications)** 

* 
**Data length**: 1 Byte 


* 
**Byte0**: Led indicators (1 is valid, 0 is invalid) 


* bit7-5: 0 


* bit4: Kana 


* bit3: Compose 


* bit2: Scrolllock 


* bit1: Capslock 


* bit0: Numlock 





---

### III. 

CHARACTERISTIC UUID: F802 

**Write: HID Gamepad User Report (complies with general format specifications)** 

* 
**Data length**: 10 Bytes 


* 
**Byte0**: Fixed as `0x22` 


* 
**Byte1**: Stick1(X) horizontal position absolute value, valid value range -127 to +127 


* 
**Byte2**: Stick1(Y) vertical position absolute value, valid value range -127 to +127 


* 
**Byte3**: Stick2(Z) horizontal position absolute value, valid value range -127 to +127 


* 
**Byte4**: Stick2(Rz) vertical position absolute value, valid value range -127 to +127 


* 
**Byte5**: Slider1(Rx) position absolute value, valid value range -127 to +127 


* 
**Byte6**: Slider2(Ry) position absolute value, valid value range -127 to +127 


* 
**Byte7**: 8 Buttons  (bit7: Button 8 ... bit0: Button 1) 


* 
**Byte8**: 8 Buttons  (bit7: Button 16 ... bit0: Button 9) 


* 
**Byte9**: Pov direction pad 4 bits, valid values 1~8, starting from upward direction clockwise 8 directions, 0 is invalid. Also contains 4 Buttons (1 is valid, 0 is invalid).


* bit7: Button Back 


* bit6: Button Home 


* bit5: Button Select 


* bit4: Button Start 


* bit3-0: Pov 4bits 





**Notify**: Ignore 

---

### IV. 

CHARACTERISTIC UUID: F803 

**Write: HID Mouse User Report (complies with general format specifications)** 

* 
**Data length**: 6 Bytes 


* 
**Byte0**: 5 mouse buttons (1 is valid, 0 is invalid) 


* bit7-5: 0 


* bit4: Button5 (Auxiliary) 


* bit3: Button4 (Auxiliary) 


* bit2: Button3 (Middle click) 


* bit1: Button2 (Right click) 


* bit0: Button1 (Left click) 




* 
**Byte1**: Mouse pointer movement X-axis relative value, lower 8 bits 


* 
**Byte2**: Mouse pointer movement X-axis relative value, higher 8 bits 


* 
*The 16-bit value composed of the above 2 bytes has a valid value range of -2047 to +2047*.




* 
**Byte3**: Mouse pointer movement Y-axis relative value, lower 8 bits 


* 
**Byte4**: Mouse pointer movement Y-axis relative value, higher 8 bits 


* 
*The 16-bit value composed of the above 2 bytes has a valid value range of -2047 to +2047*.




* 
**Byte5**: Mouse pointer movement Z-axis (scroll wheel) relative value, valid value range -127 to +127 



**Notify**: Ignore 

---

V. CHARACTERISTIC UUID: F804 

**Write: 1. HID Pen User Report** 

* 
**Data length**: 6 Bytes 


* 
**Byte0**: Fixed as `0x41` 


* 
**Byte1**: 3 function keys (1 is valid, 0 is invalid) 


* bit7-3: 0 


* bit2: Barrel 


* bit1: Tip Switch 


* bit0: In Range 




* 
**Byte2**: X-axis absolute value, lower 8 bits 


* 
**Byte3**: X-axis absolute value, higher 8 bits 


* 
*The 16-bit value composed of the above 2 bytes has a valid value range of 0-3840, corresponding to the screen's horizontal resolution*.




* 
**Byte4**: Y-axis absolute value, lower 8 bits 


* 
**Byte5**: Y-axis absolute value, higher 8 bits 


* 
*The 16-bit value composed of the above 2 bytes has a valid value range of 0-2160, corresponding to the screen's vertical resolution*.





**Write: 2. HID Consumer User Report** 

* 
**Data length**: 4 Bytes 


* 
**Byte0**: Fixed as `0x42` 


* 
**Byte1**: Number key group 4 bits (valid values 1~10, 0 is invalid), 4 function keys (1 is valid, 0 is invalid) 


* bit7: Volume + 


* bit6: Volume - 


* bit5: Channel + 


* bit4: Channel - 


* bit3-0: Number (Number key group) 4bits 




* 
**Byte2**: Function key group 6 bits, camera function keys 2 bits 


* bit7: Camera Shutter 


* bit6: Camera AutoFocus 


* bit5-0: Function key group 6bits (valid value range 1-47, see table below) 





| Value | Function | Value | Function | Value | Function |
| --- | --- | --- | --- | --- | --- |
| 1 | Assign | 17 | WWW | 33 | Copy |
| 2/3 | Play/Pause | 18/19 | Telephone/Menu | 34/35 | Cut/Paste |
| 4 | Record | 20 | MenuPick | 36 | Select All |
| 5 | Fast Forward | 21 | Brightness+ | 37 | Find |
| 6 | Fast Rewind | 22 | Brightness- | 38 | Search |
| 7 | Scan Next | 23 | Mail | 39 | Goto |
| 8 | Scan Prev | 24 | Address Book | 40 | Home |
| 9 | Stop | 25 | Calculator | 41 | Back |
| 10 | Eject | 26 | Machine Browser | 42 | Forward |
| 11 | Repeat | 27 | Internet Browser | 43 | Stop |
| 12 | Stop/Eject | 28 | Media | 44 | Refresh |
| 13 | Play/Pause | 29 | Terminal Lock | 45 | Bookmarks |
| 14 | Mute | 30 | Calendar | 46 | Show Windows |
| 15 | Power | 31 | Logon/Logoff | 47 | Show Apps |
| 16 | Camera Access | 32 | Undo |  |  |
| <br>(Table Data Source: )

 |  |  |  |  |  |

* 
**Byte3**: 8 function keys (1 is valid, 0 is invalid) 


* bit7: Dpad Left 


* bit6: Dpad Right 


* bit5: Dpad Down 


* bit4: Dpad Up 


* bit3: Speaker Mute 


* bit2: System Wake up 


* bit1: System Sleep 


* bit0: Power Down 





**Notify**: Ignore 

---

### VI. 

CHARACTERISTIC UUID: F80F 

* 
**Write**: Private data transmission instruction, not yet open, please do not use 


* 
**Data length**: < 20 Bytes 




* 
**Notify**: Private data transmission instruction, not yet open, please do not use 


* 
**Data length**: < 20 Bytes 





---

Appendix 1: Keyboard Usage ID Table (partial) 

| Key Name | Decimal Value | US/Ansi Character or Function Key |
| --- | --- | --- |
| KB_a_and_A | 4 | "a", "A" |
| KB_b_and_B | 5 | "b", "B" |
| KB_c_and_C | 6 | "c", "C" |
| KB_d_and_D | 7 | "d", "D" |
| KB_e_and_E | 8 | "e", "E" |
| KB_f_and_F | 9 | "f", "F" |
| KB_g_and_G | 10 | "g", "G" |
| KB_h_and_H | 11 | "h", "H" |
| KB_i_and_I | 12 | "i", "I" |
| KB_j_and_J | 13 | "j", "J" |
| KB_k_and_K | 14 | "k", "K" |
| KB_l_and_L | 15 | "l", "L" |
| KB_m_and_M | 16 | "m", "M" |
| KB_n_and_N | 17 | "n", "N" |
| KB_o_and_O | 18 | "o", "O" |
| KB_p_and_P | 19 | "p", "P" |
| KB_q_and_Q | 20 | "q", "Q" |
| KB_r_and_R | 21 | "r", "R" |
| KB_s_and_S | 22 | "s", "S" |
| KB_t_and_T | 23 | "t", "T" |
| KB_u_and_U | 24 | "u", "U" |
| KB_v_and_V | 25 | "v", "V" |
| KB_w_and_W | 26 | "w", "W" |
| KB_x_and_X | 27 | "x", "X" |
| KB_y_and_Y | 28 | "y", "Y" |
| KB_z_and_Z | 29 | "z", "Z" |
| KB_1_and_Symbol | 30 | "1", "!" |
| KB_2_and_Symbol | 31 | "2", "@" |
| KB_3_and_Symbol | 32 | "3", "#" |
| KB_4_and_Symbol | 33 | "4", "$" |
| KB_5_and_Symbol | 34 | "5", "%" |
| KB_6_and_Symbol | 35 | "6", "^" |
| KB_7_and_Symbol | 36 | "7", "&" |
| KB_8_and_Symbol | 37 | "8", "*" |
| KB_9_and_Symbol | 38 | "9", "(" |
| KB_0_and_Symbol | 39 | "0", ")" |
| KB_Enter | 40 | "Enter" |
| KB_Escape | 41 | "Esc" |
| KB_Backspace | 42 | "Back Space" |
| KB_Tab | 43 | "Tab" |
| KB_Spacebar | 44 | "Space Bar" |
| KB_Minus | 45 | "-", "_" |
| KB_Equal_and_Plus | 46 | "=", "+" |
| KB_Bracket_Left | 47 | "[", "{" |
| KB_Bracket_Right | 48 | "]", "}" |
| KB_Slash | 49 | "\", "|" |
| KB_NonUs_Hashtag | 50 | (Non-US/Ansi) |
| KB_Semicolon | 51 | ";", ":" |
| KB_Quotation | 52 | "'", """ |
| KB_GraveAccent | 53 | "`", "~" |
| KB_Comma | 54 | ",", "<" |
| KB_Period | 55 | ".", ">" |
| KB_Division | 56 | "/", "?" |
| KB_CapsLock | 57 | "Caps Lock" |
| KB_F1 to KB_F12 | 58-69 | "F1" to "F12" |
| KB_PrintScreen | 70 | "Print Screen" |
| KB_ScrollLock | 71 | "Scroll Lock" |
| KB_Pause | 72 | "Pause" |
| KB_Insert | 73 | "Ins" |
| KB_Home | 74 | "Home" |
| KB_PageUp | 75 | "Page Up" |
| KB_Delete | 76 | "Del" |
| KB_End | 77 | "End" |
| KB_PageDown | 78 | "Page Down" |
| KB_RightArrow | 79 | "Arrow Right" |
| KB_LeftArrow | 80 | "Arrow Left" |
| KB_DownArrow | 81 | "Arrow Down" |
| KB_UpArrow | 82 | "Arrow Up" |
| KP_NumLock_and_Clear | 83 | "N/L" |
| KP_Division | 84 | "/" |
| KP_Multiply | 85 | "*" |
| KP_Minus | 86 | "-" |
| KP_Plus | 87 | "+" |
| KP_Enter | 88 | "Enter" |
| KP_1_and_End to KP_9_and_PageUp | 89-97 | Numpad 1-9 |
| KP_0_and_Insert | 98 | "0", "Ins" |
| KP_Period_and_Delete | 99 | ".", "Del" |
| KB_NonUs_Slash_Right | 100 | (Non-US/Ansi) |
| KB_PC_Application | 101 | "Menu" |
| KB_Power | 102 | (Not a physical key) |
| KP_Equal | 103 | "=" |
| KB_MAC_F13 to KB_MAC_F15 | 104-106 | "F13" to "F15" |
| <br>(Table Data Source: )

 |  |  |

---

### Examples

**Example 1: Keyboard Operation Report (UUID: F801)** 

1. User presses Win+Shift+s (Win10 screenshot shortcut) as an example. 

| User Operation | Report Data |
| --- | --- |
| Press right Win key | `80H 00H 00H 00H 00H 00H 00H 00H` |
| Press right Shift key | `A0H 00H 00H 00H 00H 00H 00H 00H` |
| Press s key | `A0H 00H 16H 00H 00H 00H 00H 00H` |
| Release s key | `A0H 00H 00H 00H 00H 00H 00H 00H` |
| Release Win & Shift keys | `00H 00H 00H 00H 00H 00H 00H 00H` |
| <br>(Table Data Source: )

 |  |

2. User presses Shift+Up arrow+Right arrow as an example. 

| User Operation | Report Data |
| --- | --- |
| Press left Shift key | `02H 00H 00H 00H 00H 00H 00H 00H` |
| Press Up arrow key | `02H 00H 52H 00H 00H 00H 00H 00H` |
| Press Right arrow key | `02H 00H 52H 4FH 00H 00H 00H 00H` |
| Release Up arrow key | `02H 00H 4FH 00H 00H 00H 00H 00H` |
| Release Right arrow key | `02H 00H 00H 00H 00H 00H 00H 00H` |
| Release Shift key | `00H 00H 00H 00H 00H 00H 00H 00H` |
| <br>(Table Data Source: )

 |  |

**Example 2: Mouse Operation Report (UUID: F803)** 

*User moves the mouse and clicks as an example.* 

| User Operation | Report Data |
| --- | --- |
| Pointer moves down-right (6,12) | `00H 06H 00H 0CH 00H 00H` |
| Pointer moves down-left (-2,3) | `00H FEH FFH 03H 00H 00H` |
| Press left mouse button | `01H 00H 00H 00H 00H 00H` |
| Release left mouse button | `00H 00H 00H 00H 00H 00H` |
| <br>(Table Data Source: )

 |  |

**Example 3: Consumer Operation Report (UUID: F804)** 

*User presses media control keys as an example.* 

| User Operation | Report Data |
| --- | --- |
| Press Play key | `42H 00H 02H 00H` |
| Release Play key | `42H 00H 00H 00H` |
| Press Volume+ key | `42H 40H 00H 00H` |
| Release Volume+ key | `42H 00H 00H 00H` |
| Press Stop key | `42H 00H 09H 00H` |
| Release Stop key | `42H 00H 00H 00H` |
| <br>(Table Data Source: )

 |  |