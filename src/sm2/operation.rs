use crate::sm2::p256_field::{Conversion, Fe, FieldElement};
use crate::sm2::FeOperation;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use num_bigint::BigUint;
use num_integer::Integer;
use num_traits::{One, Zero};
use std::io::Cursor;

impl Conversion for Fe {
    fn fe_to_bigunit(&self) -> BigUint {
        let mut ret: Vec<u8> = Vec::new();
        for i in 0..8 {
            ret.write_u32::<BigEndian>(self[i]).unwrap();
        }
        BigUint::from_bytes_be(&ret[..])
    }

    fn bigunit_fe(&self) -> Fe {
        unimplemented!()
    }
}

impl Conversion for BigUint {
    fn fe_to_bigunit(&self) -> BigUint {
        unimplemented!()
    }

    fn bigunit_fe(&self) -> Fe {
        let v = self.to_bytes_be();
        let mut num_v = [0u8; 32];
        num_v[32 - v.len()..32].copy_from_slice(&v[..]);
        let mut elem = [0u32; 8];
        let mut c = Cursor::new(num_v);
        for i in 0..8 {
            let x = c.read_u32::<BigEndian>().unwrap();
            elem[i] = x;
        }
        elem
    }
}

impl FeOperation for Fe {
    fn mod_add(&self, other: &Self, modulus: &Self) -> Self {
        let (raw_sum, carry) = add_raw(self, other);
        if carry == 1 || raw_sum >= *modulus {
            let (sum, _borrow) = sub_raw(&raw_sum, &modulus);
            sum
        } else {
            raw_sum
        }
    }

    fn mod_add_number(&self, other: u64, modulus: &Self) -> Self {
        let mut arr: [u32; 8] = [0; 8];
        arr[7] = (other & 0xffff_ffff) as u32;
        arr[6] = (other >> 32) as u32;
        self.mod_add(&arr, modulus)
    }

    fn mod_sub(&self, other: &Self, modulus: &Self) -> Self {
        let (raw_diff, borrow) = sub_raw(&self, other);
        if borrow {
            let (modulus_complete, _) = sub_raw(&[0; 8], &modulus);
            let (diff, _borrow) = sub_raw(&raw_diff, &modulus_complete);
            diff
        } else {
            raw_diff
        }
    }

    fn mod_mul(&self, other: &Self, modulus: &Self) -> Self {
        let raw_prod = mul_raw(self, other);
        fast_reduction(&raw_prod, &modulus)
    }

    fn inv(&self, modulus: &Self) -> Self {
        let mut ru = *self;
        let mut rv = *modulus;
        let mut ra = FieldElement::from_number(1).inner;
        let mut rc = [0; 8];
        while ru != [0; 8] {
            if ru[7] & 0x01 == 0 {
                ru = ru.right_shift(0);
                if ra[7] & 0x01 == 0 {
                    ra = ra.right_shift(0);
                } else {
                    let (sum, car) = add_raw(&ra, &modulus);
                    ra = sum.right_shift(car);
                }
            }

            if rv[7] & 0x01 == 0 {
                rv = rv.right_shift(0);
                if rc[7] & 0x01 == 0 {
                    rc = rc.right_shift(0);
                } else {
                    let (sum, car) = add_raw(&rc, &modulus);
                    rc = sum.right_shift(car);
                }
            }

            if ru >= rv {
                ru = ru.mod_sub(&rv, &modulus);
                ra = ra.mod_sub(&rc, &modulus);
            } else {
                rv = rv.mod_sub(&ru, &modulus);
                rc = rc.mod_sub(&ra, &modulus);
            }
        }
        rc
    }

    fn right_shift(&self, carry: u32) -> Self {
        let mut ret = [0; 8];
        let mut carry = carry;
        let mut i = 0;
        while i < 8 {
            ret[i] = (carry << 31) + (self[i] >> 1);
            carry = self[i] & 0x01;
            i += 1;
        }
        ret
    }
}

#[inline(always)]
const fn mul_raw(a: &Fe, b: &Fe) -> [u32; 16] {
    let mut local: u64 = 0;
    let mut carry: u64 = 0;
    let mut ret: [u32; 16] = [0; 16];
    let mut ret_idx = 0;
    while ret_idx < 15 {
        let index = 15 - ret_idx;
        let mut a_idx = 0;
        while a_idx < 8 {
            if a_idx > ret_idx {
                break;
            }
            let b_idx = ret_idx - a_idx;
            if b_idx < 8 {
                let (hi, lo) = u32_mul(a[7 - a_idx], b[7 - b_idx]);
                local += lo;
                carry += hi;
            }
            a_idx += 1;
        }
        carry += local >> 32;
        local &= 0xffff_ffff;
        ret[index] = local as u32;
        local = carry;
        carry = 0;
        ret_idx += 1;
    }
    ret[0] = local as u32;
    ret
}

#[inline(always)]
const fn u32_mul(a: u32, b: u32) -> (u64, u64) {
    let uv = (a as u64) * (b as u64);
    let u = uv >> 32;
    let v = uv & 0xffff_ffff;
    (u, v)
}

// a quick algorithm to reduce elements on SCA-256 field
// Reference:
// http://ieeexplore.ieee.org/document/7285166/ for details
// 国密SM2的快速约减算法详细描述
// S0 = (m7, m6, m5, m4, m3, m2, m1, m0)
// S1 = (m15, 0, 0, 0, 0, 0, m15, m14)
// S2 = (m14, 0, 0, 0, 0, 0, m14, m13)
// S3 = (m13, 0, 0, 0, 0, 0, 0, m15)
// S4 = (m12, 0, m15, m14, m13, 0, 0, m15)
// S5 = (m15, m15, m14, m13, m12, 0, m11, m10)
// S6 = (m11, m14, m13, m12, m11, 0, m10, m9)
// S7 = (m10, m11, m10, m9, m8, 0, m13, m12)
// S8 = (m9, 0, 0, m15, m14, 0, m9, m8)
// S9 = (m8, 0, 0, 0, m15, 0, m12, m11)
#[inline(always)]
pub fn fast_reduction(a: &[u32; 16], modulus: &[u32; 8]) -> [u32; 8] {
    let mut s: [[u32; 8]; 10] = [[0; 8]; 10];
    let mut m: [u32; 16] = [0; 16];

    let mut i = 0;
    while i < 16 {
        m[i] = a[15 - i];
        i += 1;
    }

    s[0] = [m[7], m[6], m[5], m[4], m[3], m[2], m[1], m[0]];
    s[1] = [m[15], 0, 0, 0, 0, 0, m[15], m[14]];
    s[2] = [m[14], 0, 0, 0, 0, 0, m[14], m[13]];
    s[3] = [m[13], 0, 0, 0, 0, 0, 0, 0];
    s[4] = [m[12], 0, m[15], m[14], m[13], 0, 0, m[15]];
    s[5] = [m[15], m[15], m[14], m[13], m[12], 0, m[11], m[10]];
    s[6] = [m[11], m[14], m[13], m[12], m[11], 0, m[10], m[9]];
    s[7] = [m[10], m[11], m[10], m[9], m[8], 0, m[13], m[12]];
    s[8] = [m[9], 0, 0, m[15], m[14], 0, m[9], m[8]];
    s[9] = [m[8], 0, 0, 0, m[15], 0, m[12], m[11]];

    let mut carry: i32 = 0;
    let mut sum = [0; 8];

    // part1: 2 * (s1+s2+s3+s4)
    let (rt, rc) = add_raw(&sum, &s[1]);
    sum = rt;
    carry += rc as i32;
    let (rt, rc) = add_raw(&sum, &s[2]);
    sum = rt;
    carry += rc as i32;
    let (rt, rc) = add_raw(&sum, &s[3]);
    sum = rt;
    carry += rc as i32;
    let (rt, rc) = add_raw(&sum, &s[4]);
    sum = rt;
    carry += rc as i32;
    let (rt, rc) = add_raw(&sum, &sum);
    sum = rt;
    carry = carry * 2 + rc as i32;

    // part2: s0+s5+s6+s7+s8+s9
    let (rt, rc) = add_raw(&sum, &s[5]);
    sum = rt;
    carry += rc as i32;
    let (rt, rc) = add_raw(&sum, &s[6]);
    sum = rt;
    carry += rc as i32;
    let (rt, rc) = add_raw(&sum, &s[7]);
    sum = rt;
    carry += rc as i32;
    let (rt, rc) = add_raw(&sum, &s[8]);
    sum = rt;
    carry += rc as i32;
    let (rt, rc) = add_raw(&sum, &s[9]);
    sum = rt;
    carry += rc as i32;
    let (rt, rc) = add_raw(&sum, &s[0]);
    sum = rt;
    carry += rc as i32;

    // part3:  m8+m9+m13+m14
    let mut part3 = [0; 8];
    let rt: u64 = u64::from(m[8]) + u64::from(m[9]) + u64::from(m[13]) + u64::from(m[14]);
    part3[5] = (rt & 0xffff_ffff) as u32;
    part3[4] = (rt >> 32) as u32;

    // part1 + part2 - part3
    let (rt, rc) = sub_raw(&sum, &part3);
    sum = rt;
    carry -= rc as i32;

    //
    while carry > 0 || sum >= *modulus {
        let (rs, rb) = sub_raw(&sum, modulus);
        sum = rs;
        carry -= rb as i32;
    }
    sum
}

#[inline(always)]
pub const fn add_raw(a: &Fe, b: &Fe) -> (Fe, u32) {
    let mut sum = [0; 8];
    let mut carry: u32 = 0;
    let mut i = 7;
    loop {
        let (t_sum, c) = adc_32(a[i], b[i], carry);
        sum[i] = t_sum;
        carry = c;
        if i == 0 {
            break;
        }
        i -= 1;
    }
    (sum, carry)
}

/// Computes `a + b + carry`, returning the result along with the new carry.
#[inline(always)]
const fn adc_32(a: u32, b: u32, carry: u32) -> (u32, u32) {
    let ret = (a as u64) + (b as u64) + (carry as u64);
    ((ret & 0xffff_ffff) as u32, (ret >> 32) as u32)
}

#[inline(always)]
const fn sub_raw(a: &Fe, b: &Fe) -> (Fe, bool) {
    let mut sum = [0; 8];
    let mut borrow = false;
    let mut j = 0;
    while j < 8 {
        let i = 7 - j;
        let t_sum: i64 = (a[i] as i64) - (b[i] as i64) - (borrow as i64);
        if t_sum < 0 {
            sum[i] = (t_sum + (1 << 32)) as u32;
            borrow = true;
        } else {
            sum[i] = t_sum as u32;
            borrow = false;
        }
        j += 1;
    }
    (sum, borrow)
}

impl FeOperation for BigUint {
    fn mod_add(&self, other: &Self, modulus: &Self) -> BigUint {
        (self + other) % modulus
    }

    fn mod_add_number(&self, other: u64, modulus: &Self) -> BigUint {
        (self + other) % modulus
    }

    fn mod_sub(&self, other: &Self, modulus: &Self) -> BigUint {
        if self >= other {
            (self - other) % modulus
        } else {
            // 负数取模
            let d = other - self;
            let e = d.div_ceil(modulus);
            e * modulus - d
        }
    }

    fn mod_mul(&self, other: &Self, modulus: &Self) -> BigUint {
        (self * other) % modulus
    }

    fn inv(&self, modulus: &Self) -> BigUint {
        let mut ru = self.clone();
        let mut rv = modulus.clone();
        let mut ra = BigUint::one();
        let mut rc = BigUint::zero();
        let rn = modulus.clone();
        while ru != BigUint::zero() {
            if ru.is_even() {
                ru >>= 1;
                if ra.is_even() {
                    ra >>= 1;
                } else {
                    ra = (ra + &rn) >> 1;
                }
            }

            if rv.is_even() {
                rv >>= 1;
                if rc.is_even() {
                    rc >>= 1;
                } else {
                    rc = (rc + &rn) >> 1;
                }
            }

            if ru >= rv {
                ru -= &rv;
                if ra >= rc {
                    ra -= &rc;
                } else {
                    ra = ra + &rn - &rc;
                }
            } else {
                rv -= &ru;
                if rc >= ra {
                    rc -= &ra;
                } else {
                    rc = rc + &rn - &ra;
                }
            }
        }
        rc
    }

    fn right_shift(&self, carry: u32) -> BigUint {
        let mut ret = self.clone();
        ret = ret >> (carry as i32);
        ret
    }
}

#[cfg(test)]
mod test_op {
    use crate::sm2::p256_ecc::P256C_PARAMS;
    use crate::sm2::p256_pre_table::PRE_TABLE_1;
    use crate::sm2::FeOperation;
    use rand::{thread_rng, Rng};

    #[test]
    fn test_mod_add() {
        let mut rng = thread_rng();
        let n: u32 = rng.gen_range(10..256);

        let modulus = &P256C_PARAMS.p;

        let p = &PRE_TABLE_1[n as usize];
        let x = p.x.to_biguint();
        let y = p.y.to_biguint();

        let ret1 = x.mod_add(&y, &modulus.to_biguint());
        let ret2 = (p.x + p.y).to_biguint();

        assert_eq!(ret2, ret1)
    }

    #[test]
    fn test_mod_sub() {
        let mut rng = thread_rng();
        let n: u32 = rng.gen_range(10..256);

        let modulus = &P256C_PARAMS.p;

        let p = &PRE_TABLE_1[n as usize];
        let x = p.x.to_biguint();
        let y = p.y.to_biguint();

        let ret1 = x.mod_sub(&y, &modulus.to_biguint());
        let ret2 = (p.x - p.y).to_biguint();

        assert_eq!(ret2, ret1)
    }

    #[test]
    fn test_mod_mul() {
        let mut rng = thread_rng();
        let n: u32 = rng.gen_range(10..256);

        let modulus = &P256C_PARAMS.p;

        let p = &PRE_TABLE_1[n as usize];
        let x = p.x.to_biguint();
        let y = p.y.to_biguint();

        let ret1 = x.mod_mul(&y, &modulus.to_biguint());
        let ret2 = (p.x * p.y).to_biguint();

        assert_eq!(ret2, ret1)
    }
}
