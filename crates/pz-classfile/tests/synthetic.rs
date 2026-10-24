//! Round trip over a hand-assembled class inside a hand-assembled jar.

use pz_classfile::{load_class, static_inits, Jar, Value};

/// A class `t/K` whose `<clinit>` runs `A = reg("id_a"); B = reg("id_b", t/O.X)`.
fn class_bytes() -> Vec<u8> {
    let mut pool: Vec<Vec<u8>> = Vec::new();
    let utf8 = |pool: &mut Vec<Vec<u8>>, s: &str| {
        let mut v = vec![1, 0, s.len() as u8];
        v.extend_from_slice(s.as_bytes());
        pool.push(v);
        pool.len() as u16
    };
    let u = |n: u16| n.to_be_bytes();
    let this_name = utf8(&mut pool, "t/K");
    pool.push([&[7u8][..], &u(this_name)].concat());
    let this_class = pool.len() as u16;
    let obj = utf8(&mut pool, "java/lang/Object");
    pool.push([&[7u8][..], &u(obj)].concat());
    let super_class = pool.len() as u16;
    let string = |pool: &mut Vec<Vec<u8>>, s: &str| {
        let i = utf8(pool, s);
        pool.push([&[8u8][..], &u(i)].concat());
        pool.len() as u16
    };
    let s_a = string(&mut pool, "id_a");
    let s_b = string(&mut pool, "id_b");
    let member = |pool: &mut Vec<Vec<u8>>, tag: u8, class: u16, name: &str, desc: &str| {
        let n = utf8(pool, name);
        let d = utf8(pool, desc);
        pool.push([&[12u8][..], &u(n), &u(d)].concat());
        let nt = pool.len() as u16;
        pool.push([&[tag][..], &u(class), &u(nt)].concat());
        pool.len() as u16
    };
    let reg1 = member(&mut pool, 10, this_class, "reg", "(Ljava/lang/String;)Lt/K;");
    let reg2 = member(&mut pool, 10, this_class, "reg", "(Ljava/lang/String;Lt/O;)Lt/K;");
    let f_a = member(&mut pool, 9, this_class, "A", "Lt/K;");
    let f_b = member(&mut pool, 9, this_class, "B", "Lt/K;");
    let o_name = utf8(&mut pool, "t/O");
    pool.push([&[7u8][..], &u(o_name)].concat());
    let o_class = pool.len() as u16;
    let o_x = member(&mut pool, 9, o_class, "X", "Lt/O;");
    let clinit = utf8(&mut pool, "<clinit>");
    let void = utf8(&mut pool, "()V");
    let code_attr = utf8(&mut pool, "Code");

    let mut code = vec![0x12, s_a as u8, 0xb8];
    code.extend(u(reg1));
    code.push(0xb3);
    code.extend(u(f_a));
    code.extend([0x12, s_b as u8, 0xb2]);
    code.extend(u(o_x));
    code.push(0xb8);
    code.extend(u(reg2));
    code.push(0xb3);
    code.extend(u(f_b));
    code.push(0xb1);

    let mut b = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 61];
    b.extend(u(pool.len() as u16 + 1));
    for c in &pool {
        b.extend(c);
    }
    b.extend(u(0x21));
    b.extend(u(this_class));
    b.extend(u(super_class));
    b.extend(u(0)); // interfaces
    b.extend(u(0)); // fields
    b.extend(u(1)); // methods
    b.extend(u(0x08));
    b.extend(u(clinit));
    b.extend(u(void));
    b.extend(u(1));
    b.extend(u(code_attr));
    b.extend(((12 + code.len()) as u32).to_be_bytes());
    b.extend(u(4));
    b.extend(u(0));
    b.extend((code.len() as u32).to_be_bytes());
    b.extend(&code);
    b.extend(u(0)); // exception table
    b.extend(u(0)); // code attributes
    b.extend(u(0)); // class attributes
    b
}

/// A zip with one stored entry.
fn jar_bytes(name: &str, data: &[u8]) -> Vec<u8> {
    let le16 = |v: u16| v.to_le_bytes();
    let le32 = |v: u32| v.to_le_bytes();
    let mut z = Vec::new();
    z.extend(le32(0x0403_4b50));
    z.extend([0u8; 14]);
    z.extend(le32(data.len() as u32));
    z.extend(le32(data.len() as u32));
    z.extend(le16(name.len() as u16));
    z.extend(le16(0));
    z.extend(name.as_bytes());
    z.extend(data);
    let cen = z.len() as u32;
    z.extend(le32(0x0201_4b50));
    z.extend([0u8; 16]);
    z.extend(le32(data.len() as u32));
    z.extend(le32(data.len() as u32));
    z.extend(le16(name.len() as u16));
    z.extend([0u8; 12]);
    z.extend(le32(0));
    z.extend(name.as_bytes());
    let cen_len = z.len() as u32 - cen;
    z.extend(le32(0x0605_4b50));
    z.extend([0u8; 6]);
    z.extend(le16(1));
    z.extend(le32(cen_len));
    z.extend(le32(cen));
    z.extend(le16(0));
    z
}

#[test]
fn evaluates_static_initializer_from_jar() {
    let jar = Jar::from_bytes(jar_bytes("t/K.class", &class_bytes())).unwrap();
    let cf = load_class(&jar, "t/K").unwrap();
    assert_eq!(cf.this_class, "t/K");
    let inits = static_inits(&cf).unwrap();
    assert_eq!(inits.len(), 2);
    assert_eq!(inits[0].field, "A");
    match &inits[0].value {
        Value::Call { name, args, .. } => {
            assert_eq!(name, "reg");
            assert_eq!(args, &vec![Value::Str("id_a".into())]);
        }
        v => panic!("{v:?}"),
    }
    match &inits[1].value {
        Value::Call { args, .. } => assert_eq!(
            args,
            &vec![Value::Str("id_b".into()), Value::Static { class: "t/O".into(), name: "X".into() }]
        ),
        v => panic!("{v:?}"),
    }
}
