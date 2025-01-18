// TODO: 使用 WebRTC 实现数据同步，对内部数据的增量修改提取也在这里实现吧
pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
