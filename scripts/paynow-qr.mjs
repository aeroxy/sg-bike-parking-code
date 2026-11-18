// Writes public/paynow.png, the chip-in sheet's PayNow QR code, from the
// PAYNOW_NUMBER environment variable. Vercel runs it as the build command, so
// the number lives in the deployment's environment, never in this public
// repository (the PNG is gitignored), and the page shows the code alone.
// Without a usable number no file is written, and the sheet offers the card
// link alone.
//
//   PAYNOW_NUMBER=81234567 bun scripts/paynow-qr.mjs
//
// The code is PayNow's flavour of EMVCo's merchant-presented QR: static (one
// code for every payment), with the amount left for the payer to type, and a
// reference that marks the transfer as a donation in the bank statement.

import { rmSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import QRCode from 'qrcode';

const OUT = fileURLToPath(new URL('../public/paynow.png', import.meta.url));

// One EMVCo field: id, two-digit length, value.
const tlv = (id, value) => id + String(value.length).padStart(2, '0') + value;

// CRC-16/CCITT-FALSE, which EMVCo codes end with.
function crc16(text) {
  let crc = 0xffff;
  for (const byte of Buffer.from(text)) {
    crc ^= byte << 8;
    for (let i = 0; i < 8; i++) crc = crc & 0x8000 ? ((crc << 1) ^ 0x1021) & 0xffff : (crc << 1) & 0xffff;
  }
  return crc.toString(16).toUpperCase().padStart(4, '0');
}

// `mobile` is +65 and eight digits.
function payload(mobile) {
  const account = tlv('00', 'SG.PAYNOW')
    + tlv('01', '0')         // the proxy is a mobile number
    + tlv('02', mobile)
    + tlv('03', '1')         // the payer may edit the amount
    + tlv('04', '20991231'); // expiry; some bank apps want one
  const body = tlv('00', '01')
    + tlv('01', '11')        // static: the same code for every payment
    + tlv('26', account)
    + tlv('52', '0000')
    + tlv('53', '702')       // SGD
    + tlv('58', 'SG')
    + tlv('59', 'NA')        // banks show the name PayNow has on record
    + tlv('60', 'Singapore')
    + tlv('62', tlv('01', 'DONATION'))
    + '6304';
  return body + crc16(body);
}

// Takes 81234567, 8123 4567, +65 8123 4567 and the like: Singapore mobile
// numbers are eight digits starting with 8 or 9.
function mobileOf(number) {
  const digits = number.replace(/\D/g, '');
  const local = digits.length === 10 && digits.startsWith('65') ? digits.slice(2) : digits;
  return /^[89]\d{7}$/.test(local) ? '+65' + local : null;
}

// Checked on every build against an independent implementation (Python's
// binascii.crc_hqx), so a broken change fails the build instead of shipping
// a code no bank app can read.
if (crc16('123456789') !== '29B1' || payload('+6591234567') !== '00020101021126500009SG.PAYNOW010100211'
    + '+6591234567030110408209912315204000053037025802SG5902NA6009Singapore62120108DONATION630472A1') {
  throw new Error('the PayNow payload no longer matches its check');
}

const number = process.env.PAYNOW_NUMBER || '';
const mobile = mobileOf(number);
if (!mobile) {
  rmSync(OUT, { force: true });
  console.log(number ? 'PAYNOW_NUMBER is no Singapore mobile number: no PayNow QR' : 'no PAYNOW_NUMBER: no PayNow QR');
} else {
  await QRCode.toFile(OUT, payload(mobile), {
    type: 'png', errorCorrectionLevel: 'M', margin: 2, scale: 8, color: { dark: '#0B0B0B', light: '#FFFFFF' },
  });
  console.log('wrote public/paynow.png');
}
