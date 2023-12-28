use anyhow::Context;
use bit_struct::*;
use feistel_rs::{feistel_decrypt, feistel_encrypt};

bit_struct! {
    pub struct Name(u32) {
        first: u13,
        last: u14,
        order: u3,
        pronouns: u2,
    }
}

// used as a aesthetic hash, not crypto
const FEISTEL_KEY: [u8; 2] = [242, 199];
const FEISTEL_ROUNDS: u32 = 42;

const FIRST_NAMES: &str = include_str!("../names.txt");
const LAST_NAMES: &str = include_str!("../surnames.txt");
const ORDER: &str = include_str!("../order.txt");
const PRONOUNS: &str = include_str!("../pronouns.txt");

impl Name {
    pub fn from_str(name: &str) -> anyhow::Result<Self> {
        let parts: Vec<&str> = name.splitn(4, ' ').collect();
        let (first, last, order, mut pronouns) = (parts[0], parts[1], parts[2], parts[3]);
        pronouns = &pronouns[1..pronouns.len() - 1];

        let first = FIRST_NAMES
            .lines()
            .position(|x| *x == *first)
            .map(|n| u13::new(n as u16))
            .context("unknown first name")?;
        let last = LAST_NAMES
            .lines()
            .position(|x| *x == *last)
            .map(|n| u14::new(n as u16))
            .context("unknown last name")?;
        let order = ORDER
            .lines()
            .position(|x| *x == *order)
            .map(|n| u3::new(n as u8))
            .context("unknown title")?;
        let pronouns = PRONOUNS
            .lines()
            .position(|x| *x == *pronouns)
            .map(|n| u2::new(n as u8))
            .context("invalid gender")?;

        // first name 14 bits
        // last name  13 bits
        // order       3 bits
        // pronouns    2 bits

        // TODO: trim tables to avoid "zach" crashes
        Ok(Name::new(
            first.unwrap(),
            last.unwrap(),
            order.unwrap(),
            pronouns.unwrap(),
        ))
    }

    pub fn from_num(name: u32) -> Self {
        let encrypted = u32::from_le_bytes(
            feistel_encrypt(&name.to_le_bytes(), &FEISTEL_KEY, FEISTEL_ROUNDS)
                .try_into()
                .unwrap(),
        );

        // This is ok because every possible value is valid (see tests)
        unsafe { Name(UnsafeStorage::new_unsafe(encrypted)) }
    }

    pub fn to_num(self) -> u32 {
        let num = self.raw();

        u32::from_le_bytes(
            feistel_decrypt(&num.to_le_bytes(), &FEISTEL_KEY, FEISTEL_ROUNDS)
                .try_into()
                .unwrap(),
        )
    }

    pub fn to_str(&mut self) -> String {
        let first = FIRST_NAMES
            .lines()
            .nth(self.first().get().value() as usize)
            .unwrap();
        let last = LAST_NAMES
            .lines()
            .nth(self.last().get().value() as usize)
            .unwrap();
        let order = ORDER
            .lines()
            .nth(self.order().get().value() as usize)
            .unwrap();
        let pronouns = PRONOUNS
            .lines()
            .nth(self.pronouns().get().value() as usize)
            .unwrap();

        format!("{first} {last} {order} ({pronouns})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_to_from(num: u32) -> u32 {
        let name = Name::from_num(num).to_str();

        Name::from_str(&name).unwrap().to_num()
    }

    #[test]
    fn middling() {
        const TEST_NUM: u32 = 6522345;
        assert_eq!(TEST_NUM, test_to_from(TEST_NUM));
    }

    #[test]
    fn small() {
        const TEST_NUM: u32 = u32::MIN;
        assert_eq!(TEST_NUM, test_to_from(TEST_NUM));
    }

    #[test]
    fn big() {
        const TEST_NUM: u32 = u32::MAX - 1;
        assert_eq!(TEST_NUM, test_to_from(TEST_NUM));
    }

    #[test]
    fn many() {
        for n in 0..1000 {
            assert_eq!(n, test_to_from(n));
        }
    }
}
