pub mod transformers;

// use std::{fmt::format, sync::Arc};

use common_utils::{
    errors::CustomResult,
    ext_traits::BytesExt,
    request::{Method, Request, RequestBuilder, RequestContent},
    types::{AmountConvertor, StringMinorUnit, StringMinorUnitForConnector},
};
use error_stack::{report, ResultExt};
use hyperswitch_domain_models::{
    router_data::{AccessToken, ConnectorAuthType, ErrorResponse, RouterData},
    router_flow_types::{
        access_token_auth::AccessTokenAuth,
        payments::{Authorize, Capture, PSync, PaymentMethodToken, Session, SetupMandate, Void},
        refunds::{Execute, RSync},
    },
    router_request_types::{
        AccessTokenRequestData, PaymentMethodTokenizationData, PaymentsAuthorizeData,
        PaymentsCancelData, PaymentsCaptureData, PaymentsSessionData, PaymentsSyncData,
        RefundsData, SetupMandateRequestData,
    },
    router_response_types::{PaymentsResponseData, RefundsResponseData},
    types::{
        PaymentsAuthorizeRouterData, PaymentsCaptureRouterData, PaymentsSyncRouterData,
        PayoutsRouterData, RefundSyncRouterData, RefundsRouterData,
    },
};
#[cfg(feature = "payouts")]
use hyperswitch_domain_models::{
    router_flow_types::payouts::{PoCancel, PoCreate, PoFulfill, PoQuote},
    router_request_types::PayoutsData,
    router_response_types::PayoutsResponseData,
};
use hyperswitch_interfaces::{
    api::{self, ConnectorCommon, ConnectorCommonExt, ConnectorIntegration, ConnectorValidation},
    configs::Connectors,
    errors,
    events::connector_api_logs::ConnectorEvent,
    types::{self, PayoutCreateType, Response},
    webhooks,
};
use masking::{ExposeInterface, Mask};
use transformers as thunes;

use crate::{constants::headers, types::ResponseRouterData, utils};

#[derive(Clone)]
pub struct Thunes {
    amount_converter: &'static (dyn AmountConvertor<Output = StringMinorUnit> + Sync),
}

impl Thunes {
    pub fn new() -> &'static Self {
        &Self {
            amount_converter: &StringMinorUnitForConnector,
        }
    }
}

impl api::Payment for Thunes {}
impl api::PaymentSession for Thunes {}
impl api::ConnectorAccessToken for Thunes {}
impl api::MandateSetup for Thunes {}
impl api::PaymentAuthorize for Thunes {}
impl api::PaymentSync for Thunes {}
impl api::PaymentCapture for Thunes {}
impl api::PaymentVoid for Thunes {}
impl api::Refund for Thunes {}
impl api::RefundExecute for Thunes {}
impl api::RefundSync for Thunes {}
impl api::PaymentToken for Thunes {}

impl ConnectorIntegration<PaymentMethodToken, PaymentMethodTokenizationData, PaymentsResponseData>
    for Thunes
{
    // Not Implemented (R)
}

impl<Flow, Request, Response> ConnectorCommonExt<Flow, Request, Response> for Thunes
where
    Self: ConnectorIntegration<Flow, Request, Response>,
{
    fn build_headers(
        &self,
        req: &RouterData<Flow, Request, Response>,
        _connectors: &Connectors,
    ) -> CustomResult<Vec<(String, masking::Maskable<String>)>, errors::ConnectorError> {
        let mut header = vec![(
            headers::CONTENT_TYPE.to_string(),
            self.get_content_type().to_string().into(),
        )];
        let mut api_key = self.get_auth_header(&req.connector_auth_type)?;
        header.append(&mut api_key);
        Ok(header)
    }
}

impl ConnectorCommon for Thunes {
    fn id(&self) -> &'static str {
        "thunes"
    }

    fn get_currency_unit(&self) -> api::CurrencyUnit {
        api::CurrencyUnit::Base
        //    TODO! Check connector documentation, on which unit they are processing the currency.
        //    If the connector accepts amount in lower unit ( i.e cents for USD) then return api::CurrencyUnit::Minor,
        //    if connector accepts amount in base unit (i.e dollars for USD) then return api::CurrencyUnit::Base
    }

    fn common_get_content_type(&self) -> &'static str {
        "application/json"
    }

    fn base_url<'a>(&self, connectors: &'a Connectors) -> &'a str {
        connectors.thunes.base_url.as_ref()
    }

    fn get_auth_header(
        &self,
        auth_type: &ConnectorAuthType,
    ) -> CustomResult<Vec<(String, masking::Maskable<String>)>, errors::ConnectorError> {
        let auth = thunes::ThunesAuthType::try_from(auth_type)
            .change_context(errors::ConnectorError::FailedToObtainAuthType)?;
        Ok(vec![(
            headers::AUTHORIZATION.to_string(),
            auth.api_key.expose().into_masked(),
        )])
    }

    fn build_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        let response: thunes::ThunesErrorResponse = res
            .response
            .parse_struct("ThunesErrorResponse")
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;

        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);

        Ok(ErrorResponse {
            status_code: res.status_code,
            code: response.code,
            message: response.message,
            reason: response.reason,
            attempt_status: None,
            connector_transaction_id: None,
        })
    }
}

impl ConnectorValidation for Thunes {
    //TODO: implement functions when support enabled
}

impl ConnectorIntegration<Session, PaymentsSessionData, PaymentsResponseData> for Thunes {
    //TODO: implement sessions flow
}

impl ConnectorIntegration<AccessTokenAuth, AccessTokenRequestData, AccessToken> for Thunes {}

impl ConnectorIntegration<SetupMandate, SetupMandateRequestData, PaymentsResponseData> for Thunes {}

impl ConnectorIntegration<Authorize, PaymentsAuthorizeData, PaymentsResponseData> for Thunes {
    fn get_headers(
        &self,
        req: &PaymentsAuthorizeRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, masking::Maskable<String>)>, errors::ConnectorError> {
        self.build_headers(req, connectors)
    }

    fn get_content_type(&self) -> &'static str {
        self.common_get_content_type()
    }

    fn get_url(
        &self,
        _req: &PaymentsAuthorizeRouterData,
        _connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        Err(errors::ConnectorError::NotImplemented("get_url method".to_string()).into())
    }

    fn get_request_body(
        &self,
        req: &PaymentsAuthorizeRouterData,
        _connectors: &Connectors,
    ) -> CustomResult<RequestContent, errors::ConnectorError> {
        let amount = utils::convert_amount(
            self.amount_converter,
            req.request.minor_amount,
            req.request.currency,
        )?;

        let connector_router_data = thunes::ThunesRouterData::from((amount, req));
        let connector_req = thunes::ThunesPaymentsRequest::try_from(&connector_router_data)?;
        Ok(RequestContent::Json(Box::new(connector_req)))
    }

    fn build_request(
        &self,
        req: &PaymentsAuthorizeRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        Ok(Some(
            RequestBuilder::new()
                .method(Method::Post)
                .url(&types::PaymentsAuthorizeType::get_url(
                    self, req, connectors,
                )?)
                .attach_default_headers()
                .headers(types::PaymentsAuthorizeType::get_headers(
                    self, req, connectors,
                )?)
                .set_body(types::PaymentsAuthorizeType::get_request_body(
                    self, req, connectors,
                )?)
                .build(),
        ))
    }

    fn handle_response(
        &self,
        data: &PaymentsAuthorizeRouterData,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<PaymentsAuthorizeRouterData, errors::ConnectorError> {
        let response: thunes::ThunesPaymentsResponse = res
            .response
            .parse_struct("Thunes PaymentsAuthorizeResponse")
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;
        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);
        RouterData::try_from(ResponseRouterData {
            response,
            data: data.clone(),
            http_code: res.status_code,
        })
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

impl ConnectorIntegration<PSync, PaymentsSyncData, PaymentsResponseData> for Thunes {
    fn get_headers(
        &self,
        req: &PaymentsSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, masking::Maskable<String>)>, errors::ConnectorError> {
        self.build_headers(req, connectors)
    }

    fn get_content_type(&self) -> &'static str {
        self.common_get_content_type()
    }

    fn get_url(
        &self,
        _req: &PaymentsSyncRouterData,
        _connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        Err(errors::ConnectorError::NotImplemented("get_url method".to_string()).into())
    }

    fn build_request(
        &self,
        req: &PaymentsSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        Ok(Some(
            RequestBuilder::new()
                .method(Method::Get)
                .url(&types::PaymentsSyncType::get_url(self, req, connectors)?)
                .attach_default_headers()
                .headers(types::PaymentsSyncType::get_headers(self, req, connectors)?)
                .build(),
        ))
    }

    fn handle_response(
        &self,
        data: &PaymentsSyncRouterData,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<PaymentsSyncRouterData, errors::ConnectorError> {
        let response: thunes::ThunesPaymentsResponse = res
            .response
            .parse_struct("thunes PaymentsSyncResponse")
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;
        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);
        RouterData::try_from(ResponseRouterData {
            response,
            data: data.clone(),
            http_code: res.status_code,
        })
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

impl ConnectorIntegration<Capture, PaymentsCaptureData, PaymentsResponseData> for Thunes {
    fn get_headers(
        &self,
        req: &PaymentsCaptureRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, masking::Maskable<String>)>, errors::ConnectorError> {
        self.build_headers(req, connectors)
    }

    fn get_content_type(&self) -> &'static str {
        self.common_get_content_type()
    }

    fn get_url(
        &self,
        _req: &PaymentsCaptureRouterData,
        _connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        Err(errors::ConnectorError::NotImplemented("get_url method".to_string()).into())
    }

    fn get_request_body(
        &self,
        _req: &PaymentsCaptureRouterData,
        _connectors: &Connectors,
    ) -> CustomResult<RequestContent, errors::ConnectorError> {
        Err(errors::ConnectorError::NotImplemented("get_request_body method".to_string()).into())
    }

    fn build_request(
        &self,
        req: &PaymentsCaptureRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        Ok(Some(
            RequestBuilder::new()
                .method(Method::Post)
                .url(&types::PaymentsCaptureType::get_url(self, req, connectors)?)
                .attach_default_headers()
                .headers(types::PaymentsCaptureType::get_headers(
                    self, req, connectors,
                )?)
                .set_body(types::PaymentsCaptureType::get_request_body(
                    self, req, connectors,
                )?)
                .build(),
        ))
    }

    fn handle_response(
        &self,
        data: &PaymentsCaptureRouterData,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<PaymentsCaptureRouterData, errors::ConnectorError> {
        let response: thunes::ThunesPaymentsResponse = res
            .response
            .parse_struct("Thunes PaymentsCaptureResponse")
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;
        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);
        RouterData::try_from(ResponseRouterData {
            response,
            data: data.clone(),
            http_code: res.status_code,
        })
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

impl ConnectorIntegration<Void, PaymentsCancelData, PaymentsResponseData> for Thunes {}

impl ConnectorIntegration<Execute, RefundsData, RefundsResponseData> for Thunes {
    fn get_headers(
        &self,
        req: &RefundsRouterData<Execute>,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, masking::Maskable<String>)>, errors::ConnectorError> {
        self.build_headers(req, connectors)
    }

    fn get_content_type(&self) -> &'static str {
        self.common_get_content_type()
    }

    fn get_url(
        &self,
        _req: &RefundsRouterData<Execute>,
        _connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        Err(errors::ConnectorError::NotImplemented("get_url method".to_string()).into())
    }

    fn get_request_body(
        &self,
        req: &RefundsRouterData<Execute>,
        _connectors: &Connectors,
    ) -> CustomResult<RequestContent, errors::ConnectorError> {
        let refund_amount = utils::convert_amount(
            self.amount_converter,
            req.request.minor_refund_amount,
            req.request.currency,
        )?;

        let connector_router_data = thunes::ThunesRouterData::from((refund_amount, req));
        let connector_req = thunes::ThunesRefundRequest::try_from(&connector_router_data)?;
        Ok(RequestContent::Json(Box::new(connector_req)))
    }

    fn build_request(
        &self,
        req: &RefundsRouterData<Execute>,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        let request = RequestBuilder::new()
            .method(Method::Post)
            .url(&types::RefundExecuteType::get_url(self, req, connectors)?)
            .attach_default_headers()
            .headers(types::RefundExecuteType::get_headers(
                self, req, connectors,
            )?)
            .set_body(types::RefundExecuteType::get_request_body(
                self, req, connectors,
            )?)
            .build();
        Ok(Some(request))
    }

    fn handle_response(
        &self,
        data: &RefundsRouterData<Execute>,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<RefundsRouterData<Execute>, errors::ConnectorError> {
        let response: thunes::RefundResponse =
            res.response
                .parse_struct("thunes RefundResponse")
                .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;
        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);
        RouterData::try_from(ResponseRouterData {
            response,
            data: data.clone(),
            http_code: res.status_code,
        })
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

impl ConnectorIntegration<RSync, RefundsData, RefundsResponseData> for Thunes {
    fn get_headers(
        &self,
        req: &RefundSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, masking::Maskable<String>)>, errors::ConnectorError> {
        self.build_headers(req, connectors)
    }

    fn get_content_type(&self) -> &'static str {
        self.common_get_content_type()
    }

    fn get_url(
        &self,
        _req: &RefundSyncRouterData,
        _connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        Err(errors::ConnectorError::NotImplemented("get_url method".to_string()).into())
    }

    fn build_request(
        &self,
        req: &RefundSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        Ok(Some(
            RequestBuilder::new()
                .method(Method::Get)
                .url(&types::RefundSyncType::get_url(self, req, connectors)?)
                .attach_default_headers()
                .headers(types::RefundSyncType::get_headers(self, req, connectors)?)
                .set_body(types::RefundSyncType::get_request_body(
                    self, req, connectors,
                )?)
                .build(),
        ))
    }

    fn handle_response(
        &self,
        data: &RefundSyncRouterData,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<RefundSyncRouterData, errors::ConnectorError> {
        let response: thunes::RefundResponse = res
            .response
            .parse_struct("thunes RefundSyncResponse")
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;
        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);
        RouterData::try_from(ResponseRouterData {
            response,
            data: data.clone(),
            http_code: res.status_code,
        })
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

#[async_trait::async_trait]
impl webhooks::IncomingWebhook for Thunes {
    fn get_webhook_object_reference_id(
        &self,
        _request: &webhooks::IncomingWebhookRequestDetails<'_>,
    ) -> CustomResult<api_models::webhooks::ObjectReferenceId, errors::ConnectorError> {
        Err(report!(errors::ConnectorError::WebhooksNotImplemented))
    }

    fn get_webhook_event_type(
        &self,
        _request: &webhooks::IncomingWebhookRequestDetails<'_>,
    ) -> CustomResult<api_models::webhooks::IncomingWebhookEvent, errors::ConnectorError> {
        Err(report!(errors::ConnectorError::WebhooksNotImplemented))
    }

    fn get_webhook_resource_object(
        &self,
        _request: &webhooks::IncomingWebhookRequestDetails<'_>,
    ) -> CustomResult<Box<dyn masking::ErasedMaskSerialize>, errors::ConnectorError> {
        Err(report!(errors::ConnectorError::WebhooksNotImplemented))
    }
}

// impl api::payouts::PayoutQuote for Thunes {}
// impl api::payouts::PayoutCreate for Thunes {}
// impl api::payouts::PayoutFulfill for Thunes {}
// impl api::payouts::PayoutEligibility for Thunes {}

impl ConnectorIntegration<PoQuote, PayoutsData, PayoutsResponseData> for Thunes {
    fn get_url(
        &self,
        _req: &RouterData<PoQuote, PayoutsData, PayoutsResponseData>,
        connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        let base_url = self.base_url(connectors);
        Ok(format!("{}v2/money-transfer/quotations", base_url))
    }

    fn get_headers(
        &self,
        req: &RouterData<PoQuote, PayoutsData, PayoutsResponseData>,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, masking::Maskable<String>)>, errors::ConnectorError> {
        self.build_headers(req, connectors)
    }

    fn get_request_body(
        &self,
        req: &RouterData<PoQuote, PayoutsData, PayoutsResponseData>,
        _connectors: &Connectors,
    ) -> CustomResult<RequestContent, errors::ConnectorError> {
        let connector_req = thunes::ThunesPayoutQuotationRequest::try_from(req)?;
        Ok(RequestContent::Json(Box::new(connector_req)))
    }

    fn build_request(
        &self,
        req: &RouterData<PoQuote, PayoutsData, PayoutsResponseData>,
        _connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        let request = RequestBuilder::new()
            .method(Method::Post)
            .url(&types::PayoutQuoteType::get_url(self, req, _connectors)?)
            .attach_default_headers()
            .headers(types::PayoutQuoteType::get_headers(self, req, _connectors)?)
            .set_body(types::PayoutQuoteType::get_request_body(
                self,
                req,
                _connectors,
            )?)
            .build();

        Ok(Some(request))
    }

    fn handle_response(
        &self,
        data: &RouterData<PoQuote, PayoutsData, PayoutsResponseData>,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<RouterData<PoQuote, PayoutsData, PayoutsResponseData>, errors::ConnectorError>
    where
        PoQuote: Clone,
        PayoutsData: Clone,
        PayoutsResponseData: Clone,
    {
        let response: thunes::ThunesPayoutQuotationResponse = res
            .response
            .parse_struct("ThunesPayoutQuotationResponse")
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;

        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);

        RouterData::try_from(ResponseRouterData {
            response,
            data: data.clone(),
            http_code: res.status_code,
        })
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

impl ConnectorIntegration<PoCreate, PayoutsData, PayoutsResponseData> for Thunes {
    fn get_url(
        &self,
        _req: &RouterData<PoCreate, PayoutsData, PayoutsResponseData>,
        _connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        let base_url = self.base_url(_connectors);
        //let quotation_res_id = _req.request.connector_payout_id.to_owned().unwrap_or("null".to_string());
        let quotation_res_id = _req
            .request
            .connector_payout_id
            .to_owned()
            .ok_or(errors::ConnectorError::MissingRequiredField { field_name: "id" })?;

        Ok(format!(
            "{}v2/money-transfer/quotations/{}/transactions",
            base_url, quotation_res_id
        ))
    }

    fn get_headers(
        &self,
        _req: &RouterData<PoCreate, PayoutsData, PayoutsResponseData>,
        _connectors: &Connectors,
    ) -> CustomResult<Vec<(String, masking::Maskable<String>)>, errors::ConnectorError> {
        self.build_headers(_req, _connectors)
    }

    fn get_request_body(
        &self,
        _req: &RouterData<PoCreate, PayoutsData, PayoutsResponseData>,
        _connectors: &Connectors,
    ) -> CustomResult<RequestContent, errors::ConnectorError> {
        let connector_req = thunes::ThunesPayoutQuotationRequest::try_from(_req)?;
        Ok(RequestContent::Json(Box::new(connector_req)))
    }

    fn build_request(
        &self,
        req: &RouterData<PoCreate, PayoutsData, PayoutsResponseData>,
        _connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        let request = RequestBuilder::new()
            .method(Method::Post)
            .url(&PayoutCreateType::get_url(self, req, _connectors)?)
            .attach_default_headers()
            .headers(PayoutCreateType::get_headers(self, req, _connectors)?)
            .set_body(PayoutCreateType::get_request_body(self, req, _connectors)?)
            .build();

        Ok(Some(request))
    }

    fn handle_response(
        &self,
        data: &RouterData<PoCreate, PayoutsData, PayoutsResponseData>,
        event_builder: Option<&mut ConnectorEvent>,
        _res: Response,
    ) -> CustomResult<RouterData<PoCreate, PayoutsData, PayoutsResponseData>, errors::ConnectorError>
    where
        PoCreate: Clone,
        PayoutsData: Clone,
        PayoutsResponseData: Clone,
    {
        let response: thunes::ThunesPayoutTransactionResponse = _res
            .response
            .parse_struct("ThunesPayoutTransactionResponse")
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;

        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);

        RouterData::try_from(ResponseRouterData {
            response,
            data: data.clone(),
            http_code: _res.status_code,
        })

        // Ok(data.clone())
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

impl ConnectorIntegration<PoFulfill, PayoutsData, PayoutsResponseData> for Thunes {
    fn get_url(
        &self,
        req: &RouterData<PoFulfill, PayoutsData, PayoutsResponseData>,
        _connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        //let auth = thunes::ThunesAuthType::try_from(_req.connector_auth_type).change
        let base_url = self.base_url(_connectors);
        let quotation_res_id = req
            .request
            .connector_payout_id
            .to_owned()
            .ok_or(errors::ConnectorError::MissingRequiredField { field_name: "id" })?;

        Ok(format!(
            "{}v2/money-transfer/transactions/{}/confirm",
            base_url, quotation_res_id
        ))
        //Ok(format!("{}v2/money-transfer/transactions/{}/confirm", base_url, req.quote_id.as_ref().unwrap() ))
    }

    fn get_headers(
        &self,
        _req: &RouterData<PoFulfill, PayoutsData, PayoutsResponseData>,
        _connectors: &Connectors,
    ) -> CustomResult<Vec<(String, masking::Maskable<String>)>, errors::ConnectorError> {
        self.build_headers(_req, _connectors)
    }

    fn get_request_body(
        &self,
        req: &RouterData<PoFulfill, PayoutsData, PayoutsResponseData>,
        _connectors: &Connectors,
    ) -> CustomResult<RequestContent, errors::ConnectorError> {
        let connector_req = thunes::ThunesPayoutTransactionRequest::try_from(req)?;
        Ok(RequestContent::Json(Box::new(connector_req)))
        // if connector_req.is_err() {
        //     Err(report!(errors::ConnectorError::ResponseHandlingFailed))
        // } else {
        //     Ok(RequestContent::Json(Box::new(connector_req)))
        // }
    }

    fn build_request(
        &self,
        req: &RouterData<PoFulfill, PayoutsData, PayoutsResponseData>,
        _connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        let request = RequestBuilder::new()
            .method(Method::Post)
            .url(&types::PayoutFulfillType::get_url(self, req, _connectors)?)
            .attach_default_headers()
            .headers(types::PayoutFulfillType::get_headers(
                self,
                req,
                _connectors,
            )?)
            .set_body(types::PayoutFulfillType::get_request_body(
                self,
                req,
                _connectors,
            )?)
            .build();

        Ok(Some(request))
    }

    fn handle_response(
        &self,
        data: &RouterData<PoFulfill, PayoutsData, PayoutsResponseData>,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<RouterData<PoFulfill, PayoutsData, PayoutsResponseData>, errors::ConnectorError>
    where
        PoFulfill: Clone,
        PayoutsData: Clone,
        PayoutsResponseData: Clone,
    {
        let response: thunes::ThunesPayoutTransactionResponse = res
            .response
            .parse_struct("ThunesPayoutTransactionResponse")
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;

        event_builder.map(|i| i.set_error_response_body(&response));
        router_env::logger::info!(connector_response=?response);

        RouterData::try_from(ResponseRouterData {
            response,
            data: data.clone(),
            http_code: res.status_code,
        })

        //Ok(data.clone())
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

impl ConnectorIntegration<PoCancel, PayoutsData, PayoutsResponseData> for Thunes {
    fn get_url(
        &self,
        req: &RouterData<PoCancel, PayoutsData, PayoutsResponseData>,
        connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        let base_url = self.base_url(connectors);
        let quotation_res_id = req
            .request
            .connector_payout_id
            .to_owned()
            .ok_or(errors::ConnectorError::MissingRequiredField { field_name: "id" })?;

        Ok(format!(
            "{}v2/money-transfer/transactions/{}/cancel",
            base_url, quotation_res_id
        ))
        //Ok(format!("{}v2/money-transfer/transactions/{}/cancel", connectors.wise.base_url, req.quote_id.as_ref().unwrap()))
    }

    fn get_headers(
        &self,
        req: &RouterData<PoCancel, PayoutsData, PayoutsResponseData>,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, masking::Maskable<String>)>, errors::ConnectorError> {
        self.build_headers(req, connectors)
    }

    fn build_request(
        &self,
        req: &RouterData<PoCancel, PayoutsData, PayoutsResponseData>,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        let request = RequestBuilder::new()
            .method(Method::Post)
            .url(&types::PayoutCancelType::get_url(self, req, connectors)?)
            .attach_default_headers()
            .headers(types::PayoutCancelType::get_headers(self, req, connectors)?)
            .build();
        Ok(Some(request))
    }

    fn handle_response(
        &self,
        data: &PayoutsRouterData<PoCancel>,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<PayoutsRouterData<PoCancel>, errors::ConnectorError> {
        let response: thunes::ThunesPayoutTransactionResponse = res
            .response
            .parse_struct("ThunesPayoutTransactionResponse")
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;

        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);

        RouterData::try_from(ResponseRouterData {
            response,
            data: data.clone(),
            http_code: res.status_code,
        })
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

// impl ConnectorIntegration<PoEligibility, PayoutsData, PayoutsResponseData> for Thunes{

// }
