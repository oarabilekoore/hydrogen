use crate::domain::document::SearchHit;
use crate::error::Result;
use crate::ports::repository::IndexRepository;

pub struct QueryService<'a> {
    repo: &'a dyn IndexRepository,
}

impl<'a> QueryService<'a> {
    pub fn new(repo: &'a dyn IndexRepository) -> Self {
        Self { repo }
    }

    pub fn search(&self, query: &str, limit: usize, mode: &str) -> Result<Vec<SearchHit>> {
        let trimmed = query.trim();
        if trimmed.chars().count() < 2 {
            return Ok(Vec::new());
        }
        self.repo.search(trimmed, limit.max(1), mode)
    }
}
