use neutron_std::types::cosmos::adminmodule::adminmodule::{MsgSubmitProposal, MsgSubmitProposalResponse};

use test_tube_ntrn::{fn_execute};
use test_tube_ntrn::module::Module;
use test_tube_ntrn::runner::Runner;

pub struct Adminmodule<'a, R: Runner<'a>> {
    runner: &'a R,
}

impl<'a, R: Runner<'a>> Module<'a, R> for Adminmodule<'a, R> {
    fn new(runner: &'a R) -> Self {
        Self { runner }
    }
}

impl<'a, R> Adminmodule<'a, R>
where
    R: Runner<'a>,
{
    fn_execute! {
        pub submit_proposal: MsgSubmitProposal  ["/cosmos.adminmodule.adminmodule.MsgSubmitProposal"] => MsgSubmitProposalResponse
    }

}
