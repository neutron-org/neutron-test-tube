use neutron_std::types::neutron::contractmanager::{
    QueryParamsRequest, QueryParamsResponse,
};
use test_tube_ntrn::fn_query;
use test_tube_ntrn::module::Module;
use test_tube_ntrn::runner::Runner;

#[allow(dead_code)]
pub struct Contractmanager<'a, R: Runner<'a>> {
    runner: &'a R,
}

impl<'a, R: Runner<'a>> Module<'a, R> for Contractmanager<'a, R> {
    fn new(runner: &'a R) -> Self {
        Self { runner }
    }
}

impl<'a, R> Contractmanager<'a, R>
where
    R: Runner<'a>,
{
    fn_query! {
        pub query_params ["/neutron.contractmanager.Query/Params"]: QueryParamsRequest => QueryParamsResponse
    }
}
